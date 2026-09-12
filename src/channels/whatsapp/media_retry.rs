//! Recovering media whose CDN URL has expired (#1488).
//!
//! A WhatsApp media URL is not permanent. Ask for a photo from a conversation
//! a few weeks old and the download fails, which the channel reported as
//! "failed to download image" and nothing else - indistinguishable, from the
//! user's side, from the bot being broken.
//!
//! The protocol has a recovery path for exactly this: a `server-error` receipt
//! asks the server to re-upload the blob and hands back a fresh `directPath`.
//! `Client::media_reupload()` implements the whole exchange, and nothing in
//! this channel had ever called it.

use wacore::download::Downloadable;
use wacore::media_retry::MediaRetryResult;
use wacore_binary::jid::Jid;
use whatsapp_rust::client::Client;
use whatsapp_rust::features::MediaReuploadRequest;

/// Who and where a piece of media came from, which the reupload receipt has to
/// name. Carried as one value so the downloaders take one extra parameter
/// rather than four.
#[derive(Clone)]
pub(crate) struct MediaContext {
    pub msg_id: String,
    pub chat: Jid,
    pub is_from_me: bool,
    pub participant: Option<Jid>,
}

/// Download media, asking the server to re-upload it once if the first attempt
/// fails.
///
/// `set_direct_path` writes the recovered path back onto a clone of the media
/// proto; the original is left alone because it belongs to the message we were
/// handed and the retry is a local concern.
///
/// Returns `None` only when both attempts failed, so a caller's existing
/// error branch stays exactly as correct as it was.
pub(crate) async fn download_with_retry<T, F>(
    client: &Client,
    media: &T,
    ctx: &MediaContext,
    kind: &str,
    set_direct_path: F,
) -> Option<Vec<u8>>
where
    T: Downloadable + Clone,
    F: Fn(&mut T, String),
{
    match client.download(media).await {
        Ok(bytes) => return Some(bytes),
        Err(e) => tracing::debug!(
            target: "whatsapp",
            error = %e,
            kind,
            "media download failed; asking the server to re-upload"
        ),
    }

    // No media key means unencrypted newsletter media, which the reupload
    // path explicitly does not serve. Nothing to retry with.
    let Some(media_key) = media.media_key() else {
        tracing::warn!(target: "whatsapp", kind, "no media key; cannot request a re-upload");
        return None;
    };
    let request = MediaReuploadRequest {
        msg_id: &ctx.msg_id,
        chat_jid: &ctx.chat,
        media_key,
        is_from_me: ctx.is_from_me,
        participant: ctx.participant.as_ref(),
    };
    let fresh_path = match client.media_reupload().request(&request).await {
        Ok(MediaRetryResult::Success { direct_path }) => direct_path,
        Ok(other) => {
            // NotFound means the server no longer holds the blob at all, which
            // is a real answer and not a transient failure: say so rather than
            // leaving the user to guess.
            tracing::warn!(
                target: "whatsapp",
                kind,
                result = ?other,
                "server declined to re-upload the media; it is gone, not merely stale"
            );
            return None;
        }
        Err(e) => {
            tracing::warn!(target: "whatsapp", error = %e, kind, "media re-upload request failed");
            return None;
        }
    };

    let mut recovered = media.clone();
    set_direct_path(&mut recovered, fresh_path);
    match client.download(&recovered).await {
        Ok(bytes) => {
            tracing::info!(
                target: "whatsapp",
                kind,
                "recovered media through a server re-upload after an expired URL"
            );
            Some(bytes)
        }
        Err(e) => {
            tracing::error!(
                target: "whatsapp",
                error = %e,
                kind,
                "media still would not download after a successful re-upload"
            );
            None
        }
    }
}
