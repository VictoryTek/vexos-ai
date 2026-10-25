//! One ACP session: start, mode pinning, prompts and permission requests.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::agent::{Agent, CLAUDE_MODE, OPENCODE_MODE, OPENCODE_MODE_OPTION};
use crate::permission::{Decision, Options, PermissionRequest, cancelled};
use crate::rpc::{Connection, Incoming, Reader, RpcError, Writer};

/// ACP protocol version this core speaks.
const PROTOCOL_VERSION: u64 = 1;

#[derive(Clone, Debug)]
pub struct Config {
    /// Working directory of the session; ACP requires it to be absolute.
    pub cwd: PathBuf,
    /// An unanswered permission request is denied after this long.
    pub permission_timeout: Duration,
    /// Limit for setup requests (initialize, session/new, the mode reset).
    pub request_timeout: Duration,
}

impl Config {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Config {
            cwd: cwd.into(),
            permission_timeout: Duration::from_secs(600),
            request_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Spawn(std::io::Error),
    Config(&'static str),
    Protocol(String),
    /// The mode could not be pinned; the session was aborted.
    ModePin(String),
    Agent {
        code: i64,
        message: String,
    },
    Timeout(String),
    Closed,
    /// The session was aborted earlier (for example, a later mode reset failed).
    Aborted,
    /// The mode is being reset; prompts wait until it holds again.
    NotPinned,
    UnknownRequest,
    /// The decision was not one of the request's choices.
    InvalidDecision,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Spawn(e) => write!(f, "cannot start the agent: {e}"),
            Error::Config(why) => f.write_str(why),
            Error::Protocol(why) => write!(f, "protocol error: {why}"),
            Error::ModePin(why) => write!(f, "mode reset failed: {why}"),
            Error::Agent { code, message } => write!(f, "agent error {code}: {message}"),
            Error::Timeout(method) => write!(f, "{method} timed out"),
            Error::Closed => f.write_str("the agent closed the connection"),
            Error::Aborted => f.write_str("the session was aborted"),
            Error::NotPinned => f.write_str("the session mode is being reset"),
            Error::UnknownRequest => f.write_str("no such permission request"),
            Error::InvalidDecision => f.write_str("that answer was not offered"),
        }
    }
}

impl std::error::Error for Error {}

impl From<RpcError> for Error {
    fn from(e: RpcError) -> Self {
        match e {
            RpcError::Remote { code, message } => Error::Agent { code, message },
            RpcError::Closed => Error::Closed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Event {
    AgentText(String),
    ToolCall(ToolCall),
    /// Answer with [`Session::decide`]; unanswered, it is denied.
    Permission(PermissionRequest),
    /// The core ended the session, with the reason.
    Aborted(String),
    /// The agent closed the connection.
    Closed,
}

pub struct Events(mpsc::UnboundedReceiver<Event>);

impl Events {
    pub async fn recv(&mut self) -> Option<Event> {
        self.0.recv().await
    }
}

struct Pending {
    rpc_id: Value,
    options: Options,
    choices: Vec<Decision>,
    reply: Option<oneshot::Sender<Decision>>,
}

struct Inner {
    agent: Agent,
    config: Config,
    conn: Arc<Connection>,
    session_id: OnceLock<String>,
    pinned: AtomicBool,
    aborted: AtomicBool,
    pending: StdMutex<HashMap<u64, Pending>>,
    next_request: AtomicU64,
    events: mpsc::UnboundedSender<Event>,
    child: StdMutex<Option<Child>>,
}

/// A running agent session. Dropping it denies anything pending and stops the
/// agent.
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// Launches `agent` (see [`Agent::command`]) and starts a pinned session.
    pub async fn spawn(agent: Agent, config: Config) -> Result<(Session, Events), Error> {
        let cmd = agent.command();
        let mut child = Command::new(cmd.program)
            .args(cmd.args)
            .envs(cmd.env.iter().copied())
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(Error::Spawn)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(Error::Protocol("agent stdio unavailable".into()));
        };
        Self::start(
            agent,
            config,
            Box::new(stdout),
            Box::new(stdin),
            Some(child),
        )
        .await
    }

    /// Starts a pinned session over an existing transport (tests, or a caller
    /// that launches the agent itself).
    pub async fn connect<R, W>(
        agent: Agent,
        config: Config,
        reader: R,
        writer: W,
    ) -> Result<(Session, Events), Error>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::start(agent, config, Box::new(reader), Box::new(writer), None).await
    }

    async fn start(
        agent: Agent,
        config: Config,
        reader: Reader,
        writer: Writer,
        child: Option<Child>,
    ) -> Result<(Session, Events), Error> {
        if !config.cwd.is_absolute() {
            return Err(Error::Config("the working directory must be absolute"));
        }
        let (events, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            agent,
            config,
            conn: Connection::new(writer),
            session_id: OnceLock::new(),
            pinned: AtomicBool::new(false),
            aborted: AtomicBool::new(false),
            pending: StdMutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            events,
            child: StdMutex::new(child),
        });

        let reading = inner.clone();
        tokio::spawn(async move {
            let handler = reading.clone();
            reading
                .conn
                .read_loop(reader, move |message| handler.on_incoming(message))
                .await;
            reading.on_closed();
        });

        match inner.handshake().await {
            Ok(()) => Ok((Session { inner }, Events(rx))),
            Err(e) => {
                inner.abort(e.to_string()).await;
                Err(e)
            }
        }
    }

    /// Sends one prompt turn and returns its stop reason. Refused unless the
    /// session is pinned.
    pub async fn prompt(&self, text: &str) -> Result<String, Error> {
        let inner = &self.inner;
        if inner.aborted.load(Ordering::SeqCst) {
            return Err(Error::Aborted);
        }
        if !inner.pinned.load(Ordering::SeqCst) {
            return Err(Error::NotPinned);
        }
        let params = json!({ "sessionId": inner.session_id(), "prompt": [{ "type": "text", "text": text }] });
        let result = inner.conn.request("session/prompt", params).await?;
        Ok(result["stopReason"].as_str().unwrap_or_default().to_owned())
    }

    /// Answers a permission request. Only a choice the request offered counts.
    pub fn decide(&self, request: u64, decision: Decision) -> Result<(), Error> {
        let mut pending = self.inner.pending.lock().unwrap();
        let entry = pending.get_mut(&request).ok_or(Error::UnknownRequest)?;
        if !entry.choices.contains(&decision) {
            return Err(Error::InvalidDecision);
        }
        let reply = entry.reply.take().ok_or(Error::UnknownRequest)?;
        let _ = reply.send(decision);
        Ok(())
    }

    /// Cancels the current turn. Pending permission requests are answered
    /// `cancelled`, as ACP requires.
    pub async fn cancel(&self) {
        let inner = &self.inner;
        for pending in inner.take_pending() {
            let _ = inner.conn.respond(pending.rpc_id, cancelled()).await;
        }
        let _ = inner
            .conn
            .notify("session/cancel", json!({ "sessionId": inner.session_id() }))
            .await;
    }

    pub fn agent(&self) -> Agent {
        self.inner.agent
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(child) = self.inner.child.lock().unwrap().as_mut() {
            let _ = child.start_kill();
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let inner = self.inner.clone();
            runtime.spawn(async move { inner.abort("session closed".into()).await });
        }
    }
}

impl Inner {
    fn session_id(&self) -> &str {
        self.session_id
            .get()
            .map(String::as_str)
            .unwrap_or_default()
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        match timeout(
            self.config.request_timeout,
            self.conn.request(method, params),
        )
        .await
        {
            Ok(result) => result.map_err(Error::from),
            Err(_) => Err(Error::Timeout(method.to_owned())),
        }
    }

    async fn handshake(&self) -> Result<(), Error> {
        // No fs or terminal capability: the agent uses its own tools, which the
        // managed policy governs.
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                    "clientInfo": { "name": "vexos-ai", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        if init["protocolVersion"].as_u64() != Some(PROTOCOL_VERSION) {
            return Err(Error::Protocol(format!(
                "unsupported protocol version {}",
                init["protocolVersion"]
            )));
        }
        let cwd = self
            .config
            .cwd
            .to_str()
            .ok_or(Error::Config("the working directory must be valid UTF-8"))?;
        let new = self
            .request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))
            .await?;
        let id = new["sessionId"]
            .as_str()
            .ok_or_else(|| Error::Protocol("session/new returned no sessionId".into()))?;
        let _ = self.session_id.set(id.to_owned());
        // Always reset, whatever the session reports: settings files can start
        // it in another mode or agent.
        self.pin().await?;
        self.pinned.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Sets the one allowed mode and checks that it took.
    async fn pin(&self) -> Result<(), Error> {
        let sid = self.session_id();
        let failed = |e: Error| Error::ModePin(e.to_string());
        match self.agent {
            Agent::Claude => {
                self.request(
                    "session/set_mode",
                    json!({ "sessionId": sid, "modeId": CLAUDE_MODE }),
                )
                .await
                .map_err(failed)?;
            }
            Agent::OpenCode => {
                let params = json!({ "sessionId": sid, "configId": OPENCODE_MODE_OPTION, "value": OPENCODE_MODE });
                let result = self
                    .request("session/set_config_option", params)
                    .await
                    .map_err(failed)?;
                let mode = opencode_mode(&result["configOptions"]);
                if mode != Some(OPENCODE_MODE) {
                    return Err(Error::ModePin(format!("agent reports mode {mode:?}")));
                }
            }
        }
        Ok(())
    }

    fn on_incoming(self: &Arc<Self>, message: Incoming) {
        match message {
            Incoming::Request { id, method, params } if method == "session/request_permission" => {
                let inner = self.clone();
                tokio::spawn(async move { inner.on_permission(id, params).await });
            }
            // fs/* and terminal/* were never advertised.
            Incoming::Request { id, .. } => {
                let inner = self.clone();
                tokio::spawn(async move {
                    inner
                        .conn
                        .respond_error(id, -32601, "method not supported")
                        .await
                });
            }
            Incoming::Notification { method, params } if method == "session/update" => {
                self.on_update(&params["update"])
            }
            Incoming::Notification { .. } => {}
        }
    }

    fn on_update(self: &Arc<Self>, update: &Value) {
        let str_of = |key: &str| update[key].as_str().map(str::to_owned);
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                if let Some(text) = update["content"]["text"].as_str() {
                    let _ = self.events.send(Event::AgentText(text.to_owned()));
                }
            }
            Some("tool_call" | "tool_call_update") => {
                if let Some(id) = str_of("toolCallId") {
                    let call = ToolCall {
                        id,
                        title: str_of("title"),
                        kind: str_of("kind"),
                        status: str_of("status"),
                    };
                    let _ = self.events.send(Event::ToolCall(call));
                }
            }
            Some("current_mode_update")
                if self.agent == Agent::Claude
                    && update["currentModeId"].as_str() != Some(CLAUDE_MODE) =>
            {
                self.repin()
            }
            Some("config_option_update")
                if self.agent == Agent::OpenCode
                    && opencode_mode(&update["configOptions"])
                        .is_some_and(|mode| mode != OPENCODE_MODE) =>
            {
                self.repin()
            }
            _ => {}
        }
    }

    /// The agent left the pinned mode mid-session: reset it, or abort.
    fn repin(self: &Arc<Self>) {
        self.pinned.store(false, Ordering::SeqCst);
        let inner = self.clone();
        tokio::spawn(async move {
            match inner.pin().await {
                Ok(()) if !inner.aborted.load(Ordering::SeqCst) => {
                    inner.pinned.store(true, Ordering::SeqCst)
                }
                Ok(()) => {}
                Err(e) => inner.abort(e.to_string()).await,
            }
        });
    }

    async fn on_permission(self: Arc<Self>, rpc_id: Value, params: Value) {
        let options = Options::from_acp(&params["options"]);
        if self.aborted.load(Ordering::SeqCst) {
            let _ = self.conn.respond(rpc_id, options.deny()).await;
            return;
        }
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        let choices = options.choices();
        self.pending.lock().unwrap().insert(
            id,
            Pending {
                rpc_id,
                options,
                choices: choices.clone(),
                reply: Some(reply),
            },
        );

        let call = &params["toolCall"];
        let request = PermissionRequest {
            id,
            tool_call_id: call["toolCallId"].as_str().map(str::to_owned),
            title: call["title"].as_str().unwrap_or_default().to_owned(),
            kind: call["kind"].as_str().map(str::to_owned),
            raw_input: call["rawInput"].clone(),
            choices,
        };
        // Nobody to ask means deny; so does no answer in time.
        let decision = match self.events.send(Event::Permission(request)) {
            Ok(()) => timeout(self.config.permission_timeout, answer)
                .await
                .ok()
                .and_then(Result::ok),
            Err(_) => None,
        };

        // Whoever takes the entry answers it; abort and cancel may have already.
        let Some(pending) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let outcome = decision
            .and_then(|d| pending.options.outcome(d))
            .unwrap_or_else(|| pending.options.deny());
        let _ = self.conn.respond(pending.rpc_id, outcome).await;
    }

    fn take_pending(&self) -> Vec<Pending> {
        self.pending
            .lock()
            .unwrap()
            .drain()
            .map(|(_, p)| p)
            .collect()
    }

    /// Ends the session: denies anything pending, closes the connection and
    /// stops the agent.
    async fn abort(&self, reason: String) {
        if self.aborted.swap(true, Ordering::SeqCst) {
            return;
        }
        self.pinned.store(false, Ordering::SeqCst);
        for pending in self.take_pending() {
            let _ = self
                .conn
                .respond(pending.rpc_id, pending.options.deny())
                .await;
        }
        self.conn.close().await;
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.start_kill();
        }
        let _ = self.events.send(Event::Aborted(reason));
    }

    /// The agent closed the connection. Pending requests can no longer be
    /// answered, so none is left to approve.
    fn on_closed(&self) {
        if self.aborted.swap(true, Ordering::SeqCst) {
            return;
        }
        self.pinned.store(false, Ordering::SeqCst);
        self.take_pending();
        let _ = self.events.send(Event::Closed);
    }
}

/// The current value of OpenCode's `mode` config option.
fn opencode_mode(config_options: &Value) -> Option<&str> {
    config_options
        .as_array()?
        .iter()
        .find(|o| o["id"].as_str() == Some(OPENCODE_MODE_OPTION))?["currentValue"]
        .as_str()
}
