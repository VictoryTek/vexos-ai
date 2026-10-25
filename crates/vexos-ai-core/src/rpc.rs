//! Newline-delimited JSON-RPC 2.0, the ACP stdio transport.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, oneshot};

pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

#[derive(Clone, Debug)]
pub(crate) enum RpcError {
    Remote { code: i64, message: String },
    Closed,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Remote { code, message } => write!(f, "agent error {code}: {message}"),
            RpcError::Closed => f.write_str("connection closed"),
        }
    }
}

/// A message the agent started: a request we must answer, or a notification.
pub(crate) enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// A result, and the position of the response among the messages read.
type Response = Result<(Value, u64), RpcError>;

pub(crate) struct Connection {
    writer: Mutex<Option<Writer>>,
    pending: StdMutex<HashMap<u64, oneshot::Sender<Response>>>,
    next_id: AtomicU64,
    /// Messages read so far, so callers can order a response against
    /// notifications.
    received: AtomicU64,
}

impl Connection {
    pub(crate) fn new(writer: Writer) -> Arc<Self> {
        Arc::new(Connection {
            writer: Mutex::new(Some(writer)),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            received: AtomicU64::new(0),
        })
    }

    async fn send(&self, message: Value) -> Result<(), RpcError> {
        let mut writer = self.writer.lock().await;
        let writer = writer.as_mut().ok_or(RpcError::Closed)?;
        let mut line = message.to_string();
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| RpcError::Closed)?;
        writer.flush().await.map_err(|_| RpcError::Closed)
    }

    pub(crate) async fn request(&self, method: &str, params: Value) -> Response {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.send(message).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        rx.await.unwrap_or(Err(RpcError::Closed))
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    pub(crate) async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            .await
    }

    pub(crate) async fn respond_error(
        &self,
        id: Value,
        code: i64,
        message: &str,
    ) -> Result<(), RpcError> {
        let error = json!({ "code": code, "message": message });
        self.send(json!({ "jsonrpc": "2.0", "id": id, "error": error }))
            .await
    }

    /// Shuts down and drops the writer, so the agent sees end of input, and
    /// fails our requests.
    pub(crate) async fn close(&self) {
        if let Some(mut writer) = self.writer.lock().await.take() {
            let _ = writer.shutdown().await;
        }
        self.fail_pending();
    }

    fn fail_pending(&self) {
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err(RpcError::Closed));
        }
    }

    /// The position of the message being handled, when called from
    /// `on_incoming`.
    pub(crate) fn received(&self) -> u64 {
        self.received.load(Ordering::SeqCst)
    }

    /// Reads until end of input. Responses resolve our requests; everything the
    /// agent starts goes to `on_incoming`. Lines that are not JSON are skipped.
    pub(crate) async fn read_loop(&self, reader: Reader, mut on_incoming: impl FnMut(Incoming)) {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            self.received.fetch_add(1, Ordering::SeqCst);
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            match (
                message.get("method").and_then(Value::as_str),
                message.get("id"),
            ) {
                (Some(method), Some(id)) => on_incoming(Incoming::Request {
                    id: id.clone(),
                    method: method.to_owned(),
                    params,
                }),
                (Some(method), None) => on_incoming(Incoming::Notification {
                    method: method.to_owned(),
                    params,
                }),
                (None, Some(id)) => self.resolve(id, &message),
                (None, None) => {}
            }
        }
        self.fail_pending();
    }

    fn resolve(&self, id: &Value, message: &Value) {
        let Some(tx) = id
            .as_u64()
            .and_then(|id| self.pending.lock().unwrap().remove(&id))
        else {
            return;
        };
        let result = match message.get("error") {
            Some(error) => Err(RpcError::Remote {
                code: error["code"].as_i64().unwrap_or(0),
                message: error["message"].as_str().unwrap_or_default().to_owned(),
            }),
            None => Ok((
                message.get("result").cloned().unwrap_or(Value::Null),
                self.received(),
            )),
        };
        let _ = tx.send(result);
    }
}
