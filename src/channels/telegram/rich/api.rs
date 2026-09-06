//! Raw Bot API client for `sendRichMessage` (Bot API 10.1, 2026-06).
//!
//! teloxide 0.17 has no binding for this method yet, so we call it directly
//! over HTTP. `InputRichMessage` takes the message as a `markdown` (or `html`)
//! string — Telegram parses it server-side into rich blocks (tables, headings,
//! nested lists, math) — so there is no block JSON to construct: we pass the
//! model's markdown straight through.

use super::mermaid;
use super::render_html::markdown_to_html_mermaid;
use crate::channels::telegram::suggest_options::enforce_button_fit;
use teloxide::types::ThreadId;

/// Send `html` as a native rich message and return the new message id
/// (#420 path A). The HTML input mode is parsed server-side into rich
/// blocks, so `<details><summary>` becomes a native RichBlockDetails
/// collapsible, which the markdown input mode cannot express.
/// `reply_markup` is optional — pass `None` for no keyboard.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_rich_html_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    html: &str,
    reply_markup: Option<&serde_json::Value>,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    let url = format!("{}/bot{token}/sendRichMessage", api_base(api_url));
    let mut body = build_body_html(chat_id, thread_id, html);
    if let Some(kb) = reply_markup {
        body["reply_markup"] = kb.clone();
    }
    let result = post_rich(&url, &body, origin, origin_detail).await?;
    result
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .map(|id| id as i32)
        .ok_or_else(|| anyhow::anyhow!("sendRichMessage ok but response carried no message_id"))
}

/// Edit an existing rich message with markdown input (#1230). Telegram's
/// `editMessageText` accepts `rich_message.markdown`; editing from the raw
/// source (rather than pre-converted HTML) keeps pipe tables native through
/// the edit, exactly like the markdown rich send path. `reply_markup` is
/// optional — pass `None` to leave the keyboard unchanged.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_rich_markdown(
    api_url: &str,
    token: &str,
    chat_id: i64,
    message_id: i32,
    markdown: &str,
    reply_markup: Option<&serde_json::Value>,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<()> {
    let url = format!("{}/bot{token}/editMessageText", api_base(api_url));
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "rich_message": { "markdown": enforce_button_fit(markdown) },
    });
    if let Some(kb) = reply_markup {
        body["reply_markup"] = kb.clone();
    }
    post_and_check(&url, &body, origin, origin_detail).await
}

/// Edit an existing rich message with HTML input (#420 path A).
/// `reply_markup` is optional — pass `None` to leave the keyboard unchanged.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_rich_html(
    api_url: &str,
    token: &str,
    chat_id: i64,
    message_id: i32,
    html: &str,
    reply_markup: Option<&serde_json::Value>,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<()> {
    let url = format!("{}/bot{token}/editMessageText", api_base(api_url));
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "rich_message": { "html": enforce_button_fit(html) },
    });
    if let Some(kb) = reply_markup {
        body["reply_markup"] = kb.clone();
    }
    post_and_check(&url, &body, origin, origin_detail).await
}

/// Lib-pass dead-code exempt: production callers went through the media
/// variant; this shape stays for the cfg(test) API suite (custom api_url).
#[allow(dead_code)]
/// Send `markdown` as a native rich message and return the new message id.
/// Used for intermediate streamed segments, which must be tracked for later
/// footer-append / dedup. Returns `Err` so the caller can fall back to HTML.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_rich_markdown_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    markdown: &str,
    reply_to: Option<i32>,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    send_rich_markdown_target_id(
        api_url,
        token,
        chat_id,
        thread_id,
        reply_to,
        markdown,
        origin,
        origin_detail,
    )
    .await
}

/// [`send_rich_markdown_id`] with an optional Telegram reply target.
/// `reply_to` is a message id in the same chat; when set the rich body
/// carries `reply_parameters` so the reply lands threaded to that message
/// (#1230).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_rich_markdown_target_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    reply_to: Option<i32>,
    markdown: &str,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    let url = format!("{}/bot{token}/sendRichMessage", api_base(api_url));
    let result = post_rich(
        &url,
        &build_body_target(chat_id, thread_id, reply_to, markdown),
        origin,
        origin_detail,
    )
    .await?;
    result
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .map(|id| id as i32)
        .ok_or_else(|| anyhow::anyhow!("sendRichMessage ok but response carried no message_id"))
}

/// How many times a 429 is waited out before giving up and letting the caller
/// fall back. One retry was not enough: Telegram hands out multi-second waits
/// under load, so a single retry lands inside the same window it was told to
/// wait for and the send is abandoned while still rate limited.
const RICH_MAX_RETRIES: u32 = 3;

/// POST `body` to `url`, treating anything other than `{"ok":true,...}` as an
/// error (surfacing Telegram's `description`). Returns the `result` object.
///
/// A 429 is retried up to `RICH_MAX_RETRIES` times, honouring the server's
/// `retry_after`. The error returned always describes the LAST attempt: the
/// previous version rebound `status`/`text`/`parsed` inside the retry block,
/// so those bindings fell out of scope and a retry that failed for a new
/// reason was reported as the rate limit that preceded it (#927).
/// Extract correlation fields for a rich-API call from the request pair
/// (#1085 P1a): method from the URL, chat/thread from the body, len and
/// hash from the payload text. Works for sends, edits and deletes alike
/// (non-send calls simply log msg=0).
fn rich_send_fields<'a>(
    url: &'a str,
    body: &serde_json::Value,
) -> (&'a str, i64, Option<i32>, usize, String) {
    let method = url.rsplit('/').next().unwrap_or("?");
    let chat_id = body
        .get("chat_id")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let thread = body
        .get("message_thread_id")
        .and_then(serde_json::Value::as_i64)
        .map(|t| t as i32);
    let text = body
        .pointer("/rich_message/markdown")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            body.pointer("/rich_message/html")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("");
    (
        method,
        chat_id,
        thread,
        text.len(),
        crate::channels::telegram::telemetry::content_hash8(text),
    )
}

async fn post_rich(
    url: &str,
    body: &serde_json::Value,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let mut attempt = 0u32;

    loop {
        // G4 (#1211): this endpoint has its own Telegram budget, so none of
        // the typing/edit/send governors sees its traffic. Paced here, at the
        // one place every rich send and edit passes through, ahead of the
        // reactive 429 handling below.
        {
            let (_, chat_id, thread, _, _) = rich_send_fields(url, body);
            crate::channels::telegram::governor::pace_rich(
                teloxide::types::ChatId(chat_id),
                thread,
            )
            .await;
        }
        let resp = client.post(url).json(body).send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

        if status.is_success()
            && parsed.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        {
            let result = parsed
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // Correlation telemetry (#1085 P1a, review F5/F8): every rich
            // send that lands gets one line with full correlation fields.
            // Edits are NOT new sends — they must not pollute the
            // duplicate-attribution grep — so they log under their own kind.
            let (method, chat_id, thread, len, hash8) = rich_send_fields(url, body);
            let is_edit = url.contains("editMessage");
            let kind = if is_edit { "rich_edit" } else { "rich_api" };
            let msg_id = result
                .get("message_id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0) as i32;
            crate::channels::telegram::telemetry::log_send_success(
                origin,
                origin_detail,
                "-",
                kind,
                method,
                chat_id,
                thread,
                msg_id,
                len,
                &hash8,
            );
            return Ok(result);
        }

        if status.as_u16() == 429 && attempt < RICH_MAX_RETRIES {
            let retry_after = parsed
                .get("parameters")
                .and_then(|p| p.get("retry_after"))
                .and_then(|r| r.as_u64())
                .unwrap_or(5);
            attempt += 1;
            crate::channels::telegram::rate_limit::wait_out(
                "rich API",
                std::time::Duration::from_secs(retry_after),
                &format!(" (attempt {attempt}/{RICH_MAX_RETRIES})"),
            )
            .await;
            continue;
        }

        // Out of retries, or an error that retrying cannot fix. Report THIS
        // attempt so the caller logs why the send actually failed.
        let desc = parsed
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&text);
        if status.as_u16() == 429 {
            tracing::warn!(
                "Rich API still rate limited after {RICH_MAX_RETRIES} retries — falling back"
            );
        }
        {
            // Correlation telemetry (#1085 P1a): a failed rich send must
            // carry the same fields a successful one does.
            let (method, chat_id, thread, len, hash8) = rich_send_fields(url, body);
            crate::channels::telegram::telemetry::log_send_failure(
                origin,
                origin_detail,
                "-",
                "rich_api",
                method,
                chat_id,
                thread,
                len,
                &hash8,
                &format!("({status}): {desc}"),
            );
        }
        anyhow::bail!("Telegram rich API error ({status}): {desc}")
    }
}

/// POST `body` and discard the result — for calls where only success matters.
async fn post_and_check(
    url: &str,
    body: &serde_json::Value,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<()> {
    post_rich(url, body, origin, origin_detail)
        .await
        .map(|_| ())
}

/// Build the `sendRichMessage` JSON request body. Split out so the request
/// shape is unit-testable without a live bot.
pub(crate) fn build_body(
    chat_id: i64,
    thread_id: Option<ThreadId>,
    markdown: &str,
    reply_to: Option<i32>,
) -> serde_json::Value {
    build_body_target(chat_id, thread_id, reply_to, markdown)
}

/// [`build_body`] with an optional Telegram reply target (#1230). When
/// `reply_to` is set, the body carries `reply_parameters: {message_id}`
/// so a rich reply lands threaded to that message.
pub(crate) fn build_body_target(
    chat_id: i64,
    thread_id: Option<ThreadId>,
    reply_to: Option<i32>,
    markdown: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "rich_message": { "markdown": enforce_button_fit(markdown) },
    });
    if let Some(t) = thread_id {
        // ThreadId wraps a MessageId(i32).
        body["message_thread_id"] = serde_json::json!(t.0.0);
    }
    if let Some(mid) = reply_to {
        body["reply_parameters"] = serde_json::json!({ "message_id": mid });
    }
    body
}

/// Build the `sendRichMessage` body with HTML input (#420 path A).
/// `InputRichMessage` accepts `markdown` or `html`; HTML is the mode that
/// can express RichBlockDetails via `<details><summary>`.
pub(crate) fn build_body_html(
    chat_id: i64,
    thread_id: Option<ThreadId>,
    html: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "rich_message": { "html": enforce_button_fit(html) },
    });
    if let Some(t) = thread_id {
        body["message_thread_id"] = serde_json::json!(t.0.0);
    }
    body
}

/// Send `markdown` with a `media` array as a native rich message
/// (Bot API 10.2+, #1044). The markdown references each image via
/// `tg://photo?id=<id>`; the `media` array maps each id to an image source.
///
/// Two delivery modes, switched per entry (#1044 media plumbing):
///
/// - `MediaEntry::url` — the legacy URL path: the source is a renderer URL
///   (e.g. mermaid.ink) that Telegram fetches server-side. Sent as a plain
///   JSON body.
/// - `MediaEntry::bytes` (the active path: pre-validated mermaid.ink render,
///   #1238) — the PNG bytes are uploaded to Telegram via multipart as
///   `attach://<id>` photo parts, so Telegram never touches a third-party
///   URL. The request must therefore be `multipart/form-data`, not JSON.
///
/// Carries `reply_parameters` when `reply_to` is set (#1230). Returns the
/// new message id. Returns `Err` on transport/API failure so the caller can
/// fall back to the HTML dialect.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_rich_markdown_media_target_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    reply_to: Option<i32>,
    markdown: &str,
    media: &[super::mermaid::MediaEntry],
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    let url = format!("{}/bot{token}/sendRichMessage", api_base(api_url));
    let body = build_body_markdown_media_target(chat_id, thread_id, reply_to, markdown, media);

    let result = if media.iter().any(|m| m.bytes.is_some()) {
        // Local render → multipart upload of the PNG bytes (attach://).
        post_rich_multipart(&url, media, &body, origin, origin_detail).await?
    } else {
        // Legacy URL path → plain JSON; Telegram refetches the URL.
        post_rich(&url, &body, origin, origin_detail).await?
    };

    result
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .map(|id| id as i32)
        .ok_or_else(|| anyhow::anyhow!("sendRichMessage ok but response carried no message_id"))
}

/// Scalar string form parts for a rich multipart request: the JSON body's
/// top-level fields Telegram needs as form parts. Split out so the part
/// shape is unit-testable without a live bot. `message_id` is edit-only:
/// `editMessageText` multipart rejects with `message to edit not found`
/// when the part is missing; `sendRichMessage` bodies carry no
/// `message_id`, so sends are unchanged.
pub(crate) fn multipart_scalar_fields(body: &serde_json::Value) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    if let Some(v) = body.get("chat_id") {
        fields.push(("chat_id".to_string(), v.to_string()));
    }
    if let Some(v) = body.get("message_id") {
        fields.push(("message_id".to_string(), v.to_string()));
    }
    if let Some(v) = body.get("message_thread_id") {
        fields.push(("message_thread_id".to_string(), v.to_string()));
    }
    if let Some(v) = body.get("reply_parameters") {
        fields.push(("reply_parameters".to_string(), v.to_string()));
    }
    if let Some(v) = body.get("rich_message") {
        fields.push(("rich_message".to_string(), v.to_string()));
    }
    fields
}

/// Build the multipart/form-data request for a `sendRichMessage` whose media
/// array references uploaded PNG bytes via `attach://<id>`. Scalar parts come
/// from [`multipart_scalar_fields`]; each byte entry becomes a file part
/// named exactly `<id>` so Telegram's `attach://<id>` reference resolves
/// (§ Bot API multipart media convention). The JSON body is still passed in
/// for correlation telemetry ([`rich_send_fields`] reads its `rich_message`
/// pointer).
fn build_multipart_form(
    media: &[super::mermaid::MediaEntry],
    body: &serde_json::Value,
) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new();
    for (name, value) in multipart_scalar_fields(body) {
        form = form.text(name, value);
    }
    for m in media {
        if let Some(bytes) = &m.bytes {
            let part = reqwest::multipart::Part::bytes(bytes.clone())
                .file_name(format!("{}.png", m.id))
                .mime_str("image/png")
                .expect("image/png is a valid mime");
            form = form.part(m.id.clone(), part);
        }
    }
    form
}

/// POST `form` (multipart, for `attach://` bytes upload) to `url`, mirroring
/// [`post_rich`]'s retry / rate-limit / telemetry semantics exactly. Accepts
/// only the media-with-bytes send; every non-`ok` response is an error that
/// surfaces Telegram's `description`.
async fn post_rich_multipart(
    url: &str,
    media: &[super::mermaid::MediaEntry],
    body: &serde_json::Value,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let mut attempt = 0u32;

    loop {
        {
            let (_, chat_id, thread, _, _) = rich_send_fields(url, body);
            crate::channels::telegram::governor::pace_rich(
                teloxide::types::ChatId(chat_id),
                thread,
            )
            .await;
        }
        // Form is not Clone, so rebuild it from media+body each attempt.
        let form = build_multipart_form(media, body);
        let resp = client.post(url).multipart(form).send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

        if status.is_success()
            && parsed.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        {
            let result = parsed
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let (method, chat_id, thread, len, hash8) = rich_send_fields(url, body);
            let msg_id = result
                .get("message_id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0) as i32;
            crate::channels::telegram::telemetry::log_send_success(
                origin,
                origin_detail,
                "-",
                "rich_api",
                method,
                chat_id,
                thread,
                msg_id,
                len,
                &hash8,
            );
            return Ok(result);
        }

        if status.as_u16() == 429 && attempt < RICH_MAX_RETRIES {
            let retry_after = parsed
                .get("parameters")
                .and_then(|p| p.get("retry_after"))
                .and_then(|r| r.as_u64())
                .unwrap_or(5);
            attempt += 1;
            crate::channels::telegram::rate_limit::wait_out(
                "rich API",
                std::time::Duration::from_secs(retry_after),
                &format!(" (attempt {attempt}/{RICH_MAX_RETRIES})"),
            )
            .await;
            continue;
        }

        let desc = parsed
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&text);
        if status.as_u16() == 429 {
            tracing::warn!(
                "Rich API (multipart) still rate limited after {RICH_MAX_RETRIES} retries — falling back"
            );
        }
        {
            let (method, chat_id, thread, len, hash8) = rich_send_fields(url, body);
            crate::channels::telegram::telemetry::log_send_failure(
                origin,
                origin_detail,
                "-",
                "rich_api",
                method,
                chat_id,
                thread,
                len,
                &hash8,
                &format!("({status}): {desc}"),
            );
        }
        anyhow::bail!("Telegram rich API error ({status}): {desc}")
    }
}

/// Build the `sendRichMessage` body with markdown input + a `media` array
/// (Bot API 10.2+, #1044). Split out so the request shape is unit-testable
/// without a live bot. Matches the validated prototype (message 1073):
/// `rich_message: {markdown, media: [{id, media: {type:"photo", media:url}}]}`.
/// Carries `reply_parameters` when `reply_to` is set (#1230).
pub(crate) fn build_body_markdown_media_target(
    chat_id: i64,
    thread_id: Option<ThreadId>,
    reply_to: Option<i32>,
    markdown: &str,
    media: &[super::mermaid::MediaEntry],
) -> serde_json::Value {
    let media_arr: Vec<serde_json::Value> = media
        .iter()
        .map(|m| {
            // Local PNG bytes upload via multipart as `attach://<id>`; URL
            // entries keep the legacy server-side fetch reference.
            let source = match (&m.bytes, &m.url) {
                (Some(_), _) => format!("attach://{}", m.id),
                (None, Some(url)) => url.clone(),
                (None, None) => String::new(),
            };
            serde_json::json!({
                "id": m.id,
                "media": { "type": "photo", "media": source },
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "rich_message": { "markdown": enforce_button_fit(markdown), "media": media_arr },
    });
    if let Some(t) = thread_id {
        body["message_thread_id"] = serde_json::json!(t.0.0);
    }
    if let Some(mid) = reply_to {
        body["reply_parameters"] = serde_json::json!({ "message_id": mid });
    }
    body
}

/// The API base with any trailing separator removed.
///
/// `Bot::api_url()` returns a parsed `Url`, and the URL spec normalises an
/// empty path to `/`, so `as_str()` yields `https://api.telegram.org/`.
/// Concatenating that produced `https://api.telegram.org//bot<token>/method`,
/// which Telegram rejects — every rich send failed and silently fell back to
/// plain HTML, so tool blocks stopped rendering as rich and completions
/// arrived as separate messages (#1117).
///
/// The tests that shipped with the parameterisation could not catch it:
/// mockito's `server.url()` has no trailing slash, so they exercised a shape
/// production never uses. teloxide builds these with `path_segments_mut()`
/// for the same reason.
fn api_base(api_url: &str) -> &str {
    api_url.trim_end_matches('/')
}

/// Send `markdown` as a native rich message, rendering any mermaid fences as
/// embedded images first (#1044). When a mermaid fence is present and the
/// feature is enabled, fences are resolved once and the message goes out via
/// the markdown dialect with a `media` array (so pipe tables stay native);
/// if that send fails (e.g. a Bot API server < 10.2 without the `media`
/// field) it falls back to the HTML dialect. With no mermaid fence it stays
/// byte-identical to the plain rich-markdown path. Returns `Err` on
/// transport failure so the caller can fall back.
pub(crate) async fn send_rich_with_mermaid(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    markdown: &str,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<()> {
    send_rich_with_mermaid_id(
        api_url,
        token,
        chat_id,
        thread_id,
        markdown,
        None,
        origin,
        origin_detail,
    )
    .await
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
/// Same as [`send_rich_with_mermaid`] but returns the new message id.
pub(crate) async fn send_rich_with_mermaid_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    markdown: &str,
    reply_to: Option<i32>,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    send_rich_with_mermaid_target_id(
        api_url,
        token,
        chat_id,
        thread_id,
        reply_to,
        markdown,
        origin,
        origin_detail,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
/// Like [`send_rich_with_mermaid_id`] but carries an optional Telegram reply
/// target (`reply_parameters`) on the rich send, so a rich reply lands
/// threaded to an existing message (#1230).
pub(crate) async fn send_rich_with_mermaid_target_id(
    api_url: &str,
    token: &str,
    chat_id: i64,
    thread_id: Option<ThreadId>,
    reply_to: Option<i32>,
    markdown: &str,
    origin: &str,
    origin_detail: &str,
) -> anyhow::Result<i32> {
    if !mermaid::should_render_mermaid(markdown) {
        return send_rich_markdown_target_id(
            api_url,
            token,
            chat_id,
            thread_id,
            reply_to,
            markdown,
            origin,
            origin_detail,
        )
        .await;
    }

    // Resolve every fence once: valid diagrams become markdown media
    // references, broken ones become legible failure blocks. Non-fence text
    // is left byte-identical.
    let (resolved, media) = mermaid::resolve_markdown_media(markdown).await;

    // All fences failed → `resolved` carries only failure blocks, no media to
    // embed; send it as plain rich markdown (no `media` field).
    if media.is_empty() {
        return send_rich_markdown_target_id(
            api_url,
            token,
            chat_id,
            thread_id,
            reply_to,
            &resolved,
            origin,
            origin_detail,
        )
        .await;
    }

    // Primary: markdown dialect + media array keeps pipe tables native.
    match send_rich_markdown_media_target_id(
        api_url,
        token,
        chat_id,
        thread_id,
        reply_to,
        &resolved,
        &media,
        origin,
        origin_detail,
    )
    .await
    {
        Ok(id) => Ok(id),
        Err(e) => {
            // Fallback: HTML dialect (Bot API < 10.2 or a media-field
            // rejection). Tables degrade there, but the message still lands.
            tracing::warn!("rich markdown+media send failed ({e}); falling back to html dialect");
            let html = markdown_to_html_mermaid(markdown).await;
            send_rich_html_id(
                api_url,
                token,
                chat_id,
                thread_id,
                &html,
                None,
                origin,
                origin_detail,
            )
            .await
        }
    }
}
