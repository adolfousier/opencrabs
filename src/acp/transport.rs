//! The NDJSON stdio pump: stdin lines in, stdout lines out, and correlation
//! for server-initiated requests (`session/request_permission`).
//!
//! Two tasks own the pipes: a reader that classifies each inbound line
//! (requests/notifications go to the dispatch channel; responses resolve
//! pending outbound calls) and a writer that serializes every outbound frame
//! through one mpsc queue. That single writer is the stdout-purity invariant
//! made structural: nothing else in the process may touch stdout.
//!
//! The type splits by role: [`Transport`] owns the inbound side (the dispatch
//! loop drains it), while [`TransportHandle`] is the clonable outbound half
//! handed to turn tasks so they can notify and call while the loop keeps
//! reading.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use super::protocol::{self, ClientMessage, RpcError};

/// Outbound request ids start at 1. Correlation is direction-separated: the
/// reader routes by message shape (responses carry no `method`), so ids only
/// need to be unique among OUR in-flight requests.
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct Pending {
    waiting: Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>,
}

struct TransportInner {
    write_tx: mpsc::UnboundedSender<Value>,
    pending: Pending,
}

/// Clonable outbound half: notifications, responses, and server→client calls.
#[derive(Clone)]
pub struct TransportHandle {
    inner: Arc<TransportInner>,
}

/// Inbound half: the dispatch loop's stream of client requests/notifications.
pub struct Transport {
    inbound_rx: mpsc::UnboundedReceiver<ClientMessage>,
    inner: Arc<TransportInner>,
}

impl Transport {
    /// Spawn the reader/writer tasks on the current runtime.
    pub fn spawn() -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<ClientMessage>();
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Value>();
        let inner = Arc::new(TransportInner {
            write_tx: write_tx.clone(),
            pending: Pending::default(),
        });

        tokio::spawn(reader_task(inbound_tx, write_tx, inner.clone()));
        tokio::spawn(writer_task(write_rx));

        Self { inbound_rx, inner }
    }

    /// The outbound handle for turn tasks and the dispatcher.
    pub fn handle(&self) -> TransportHandle {
        TransportHandle {
            inner: self.inner.clone(),
        }
    }

    /// Next inbound request or notification; None when stdin closed.
    pub async fn next(&mut self) -> Option<ClientMessage> {
        self.inbound_rx.recv().await
    }
}

impl TransportHandle {
    /// Queue one frame for stdout. Never blocks the caller on IO.
    pub fn send(&self, frame: Value) {
        if self.inner.write_tx.send(frame).is_err() {
            tracing::warn!("acp transport: writer gone, dropping frame");
        }
    }

    /// Respond to an inbound request.
    pub fn respond(&self, id: Value, result: Value) {
        self.send(protocol::result_response(id, result));
    }

    /// Respond to an inbound request with an error.
    pub fn respond_error(&self, id: Value, code: i64, message: impl Into<String>) {
        self.send(protocol::error_response(id, code, message));
    }

    /// Send a notification (no response expected).
    pub fn notify(&self, method: &str, params: Value) {
        self.send(protocol::notification(method, params));
    }

    /// Server-initiated request: register a pending slot, send the frame, and
    /// await the client's response. Errors when the client responds with an
    /// error or the connection drops mid-wait.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.waiting.lock().await.insert(id, tx);
        self.send(protocol::request_frame(id, method, params));
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(anyhow!("client error {}: {}", err.code, err.message)),
            Err(_) => Err(anyhow!("connection closed awaiting {method} response")),
        }
    }
}

/// Read stdin lines forever. Responses resolve pending outbound calls;
/// requests/notifications flow to the dispatch channel. A parse failure is
/// answered with a JSON-RPC error frame (id null) and the loop continues —
/// one bad line must not kill the server.
async fn reader_task(
    inbound_tx: mpsc::UnboundedSender<ClientMessage>,
    write_tx: mpsc::UnboundedSender<Value>,
    inner: Arc<TransportInner>,
) {
    // One JSON-RPC frame per line. lines()/next_line() grow their buffer
    // until a newline arrives, so a stream without one (runaway writer,
    // binary garbage) inflates memory without bound. read_capped_line keeps
    // the ceiling at MAX_LINE_BYTES: generous for prompts legitimately
    // carrying hundreds of thousands of tokens, finite for a stuck stream.
    // An oversized frame is answered as a parse error and the stream
    // resyncs at the next newline instead of killing the server.
    let mut reader = BufReader::new(tokio::io::stdin());
    loop {
        match read_capped_line(&mut reader, MAX_LINE_BYTES).await {
            LineRead::Line(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                if line.trim().is_empty() {
                    continue;
                }
                match protocol::parse_line(&line) {
                    Ok(ClientMessage::Response { id, result }) => {
                        if let Some(tx) = inner.pending.waiting.lock().await.remove(&id) {
                            let _ = tx.send(result);
                        } else {
                            tracing::debug!("acp transport: response for unknown id {id}");
                        }
                    }
                    Ok(msg) => {
                        if inbound_tx.send(msg).is_err() {
                            return; // dispatch loop gone — shut down
                        }
                    }
                    Err((id, err)) => {
                        // Malformed line: answer with a JSON-RPC error frame
                        // (id null for parse failures) and keep serving.
                        let frame = protocol::error_response(id, err.code, err.message);
                        if write_tx.send(frame).is_err() {
                            return;
                        }
                    }
                }
            }
            LineRead::Oversized => {
                let frame = protocol::error_response(
                    serde_json::Value::Null,
                    protocol::PARSE_ERROR,
                    format!("inbound line exceeds {MAX_LINE_BYTES} bytes"),
                );
                if write_tx.send(frame).is_err() {
                    return;
                }
            }
            LineRead::Eof => {
                fail_pending(&inner, "client closed stdin").await;
                return;
            }
            LineRead::Failed(e) => {
                tracing::warn!("acp transport: stdin read failed: {e}");
                fail_pending(&inner, "stdin read failed").await;
                return;
            }
        }
    }
}

/// Hard ceiling for one inbound JSON-RPC line. Sized far above any honest
/// prompt (64 MiB); its only job is bounding a newline-less runaway stream.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Outcome of one capped inbound frame read.
#[derive(Debug)]
pub(crate) enum LineRead {
    /// Complete frame, newline consumed.
    Line(Vec<u8>),
    /// stdin closed with nothing buffered past the last newline.
    Eof,
    /// OS-level read failure.
    Failed(std::io::Error),
    /// More than `cap` bytes arrived without a newline; the remainder was
    /// drained up to the next frame boundary.
    Oversized,
}

/// Read one newline-terminated frame, refusing to hold more than `cap`
/// bytes. The bound is checked while growing chunk by chunk — a length
/// check after the fact would be too late, since `read_until` would already
/// have sized its buffer to fit the entire runaway line.
pub(crate) async fn read_capped_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    cap: usize,
) -> LineRead {
    let mut line: Vec<u8> = Vec::new();
    let mut overflowing = false;
    loop {
        let chunk = match reader.fill_buf().await {
            Ok(c) => c,
            Err(e) => return LineRead::Failed(e),
        };
        if chunk.is_empty() {
            // EOF while draining an oversized frame: that frame was dropped,
            // so surface Oversized once (the drain found its end); a fresh
            // call on the closed stream reports Eof.
            return if overflowing {
                LineRead::Oversized
            } else if line.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Line(line)
            };
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                if !overflowing {
                    line.extend_from_slice(&chunk[..pos]);
                }
                reader.consume(pos + 1);
                // A frame completed inside one big fill never trips the
                // growth check in the None branch; judge the finished line
                // against the cap here as well.
                return if overflowing || line.len() > cap {
                    LineRead::Oversized
                } else {
                    LineRead::Line(line)
                };
            }
            None => {
                if !overflowing {
                    line.extend_from_slice(chunk);
                    if line.len() > cap {
                        line.clear();
                        overflowing = true;
                    }
                }
                let n = chunk.len();
                reader.consume(n);
            }
        }
    }
}

/// Fail every in-flight server-initiated call. Once the inbound stream is
/// gone no response can resolve them: awaiting turn tasks (an approval that
/// never got its answer) must deny immediately instead of hanging until the
/// timeout/cancel backstop.
async fn fail_pending(inner: &TransportInner, reason: &str) {
    let mut waiting = inner.pending.waiting.lock().await;
    for (_, tx) in waiting.drain() {
        let _ = tx.send(Err(RpcError {
            code: -32000,
            message: reason.to_string(),
        }));
    }
}

/// Serialize outbound frames onto stdout, one JSON value per line.
async fn writer_task(mut rx: mpsc::UnboundedReceiver<Value>) {
    let mut stdout = tokio::io::stdout();
    while let Some(frame) = rx.recv().await {
        let mut line = match serde_json::to_string(&frame) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("acp transport: frame failed to serialize: {e}");
                continue;
            }
        };
        line.push('\n');
        if stdout.write_all(line.as_bytes()).await.is_err() {
            return;
        }
        if stdout.flush().await.is_err() {
            return;
        }
    }
}
