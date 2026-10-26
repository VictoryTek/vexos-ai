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

use crate::agent::{Agent, MODE_OPTION};
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
    /// The mode the core has confirmed, for display only: after the initial
    /// pin, after a reset that was acknowledged and is still the newest, and
    /// for an allowed report while no reset runs (or read after the reset's
    /// acknowledgement). Never for a disallowed or unreadable report.
    Mode(String),
    /// Answer with [`Session::decide`]; unanswered, it is denied.
    Permission(PermissionRequest),
    /// Permission request `id` went unanswered for `permission_timeout` and
    /// was denied. Sent after the deny went out, for display only.
    TimedOut(u64),
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
    /// Bumped by every disallowed mode report; a reset only re-allows prompts
    /// if no newer report arrived while it ran.
    mode_reports: AtomicU64,
    mode_view: StdMutex<ModeView>,
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
            mode_reports: AtomicU64::new(0),
            mode_view: StdMutex::new(ModeView::default()),
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
        let (result, _) = inner.conn.request("session/prompt", params).await?;
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
        self.request_at(method, params)
            .await
            .map(|(result, _)| result)
    }

    /// The result, and where the response came among the agent's messages.
    async fn request_at(&self, method: &str, params: Value) -> Result<(Value, u64), Error> {
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
        // A disallowed report during the pin starts its own reset; then that
        // reset opens the session, and prompts are refused until it holds.
        let reports = self.mode_reports.load(Ordering::SeqCst);
        let ack = self.pin().await?;
        self.pin_held(ack, reports);
        Ok(())
    }

    /// Sets the start mode and checks that it took. Returns where the
    /// acknowledgement came among the agent's messages.
    async fn pin(&self) -> Result<u64, Error> {
        let sid = self.session_id();
        let start = self.agent.start_mode();
        let failed = |e: Error| Error::ModePin(e.to_string());
        match self.agent {
            Agent::Claude => {
                let (_, ack) = self
                    .request_at(
                        "session/set_mode",
                        json!({ "sessionId": sid, "modeId": start }),
                    )
                    .await
                    .map_err(failed)?;
                Ok(ack)
            }
            Agent::OpenCode => {
                let params = json!({ "sessionId": sid, "configId": MODE_OPTION, "value": start });
                let (result, ack) = self
                    .request_at("session/set_config_option", params)
                    .await
                    .map_err(failed)?;
                let report = mode_report(&result["configOptions"]);
                if report != ModeReport::Mode(start) {
                    return Err(Error::ModePin(format!("agent reports {report:?}")));
                }
                Ok(ack)
            }
        }
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
            // Both are mode reports, from either agent. A report that can't be
            // read counts as a disallowed mode.
            Some("current_mode_update") => {
                let report = match update["currentModeId"].as_str() {
                    Some(mode) => ModeReport::Mode(mode),
                    None => ModeReport::Malformed,
                };
                self.on_mode_report(report)
            }
            Some("config_option_update") => {
                self.on_mode_report(mode_report(&update["configOptions"]))
            }
            _ => {}
        }
    }

    fn on_mode_report(self: &Arc<Self>, report: ModeReport) {
        let at = self.conn.received();
        let mut view = self.mode_view.lock().unwrap();
        match report {
            ModeReport::NotReported => {}
            // Shown now, or once the running reset is acknowledged if it came
            // after that.
            ModeReport::Mode(mode) if self.agent.allowed_modes().contains(&mode) => {
                if view.confirmed {
                    self.show_mode(mode);
                } else {
                    view.deferred = Some((mode.to_owned(), at));
                }
            }
            // Under the lock, so a reset finishing now can't show its mode.
            ModeReport::Mode(_) | ModeReport::Malformed => {
                view.confirmed = false;
                view.deferred = None;
                self.repin();
            }
        }
    }

    /// A pin was acknowledged at `ack`, and `reports` disallowed reports had
    /// been seen when it began. Unless a newer one arrived since, reopen
    /// prompts, show the start mode, then any allowed report read after the
    /// acknowledgement. Under the lock a disallowed report takes, so one can't
    /// land between the check and the reopening.
    fn pin_held(&self, ack: u64, reports: u64) {
        let mut view = self.mode_view.lock().unwrap();
        if self.aborted.load(Ordering::SeqCst)
            || self.mode_reports.load(Ordering::SeqCst) != reports
        {
            return;
        }
        self.pinned.store(true, Ordering::SeqCst);
        view.confirmed = true;
        self.show_mode(self.agent.start_mode());
        if let Some((mode, at)) = view.deferred.take()
            && at > ack
        {
            self.show_mode(&mode);
        }
    }

    fn show_mode(&self, mode: &str) {
        let _ = self.events.send(Event::Mode(mode.to_owned()));
    }

    /// The agent reported a disallowed mode: reset it to the start mode, or
    /// abort. Prompts are refused until the reset holds. Called with the
    /// `mode_view` lock held, which `pin_held` relies on.
    fn repin(self: &Arc<Self>) {
        self.pinned.store(false, Ordering::SeqCst);
        let report = self.mode_reports.fetch_add(1, Ordering::SeqCst) + 1;
        let inner = self.clone();
        tokio::spawn(async move {
            match inner.pin().await {
                // Unless a newer report's reset decides, or the session is gone.
                Ok(ack) => inner.pin_held(ack, report),
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
        let (decision, timed_out) = match self.events.send(Event::Permission(request)) {
            Ok(()) => match timeout(self.config.permission_timeout, answer).await {
                Ok(answer) => (answer.ok(), false),
                Err(_) => (None, true),
            },
            Err(_) => (None, false),
        };

        // Whoever takes the entry answers it; abort and cancel may have already.
        let Some(pending) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let outcome = decision
            .and_then(|d| pending.options.outcome(d))
            .unwrap_or_else(|| pending.options.deny());
        let _ = self.conn.respond(pending.rpc_id, outcome).await;
        if timed_out {
            let _ = self.events.send(Event::TimedOut(id));
        }
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
        self.mode_view.lock().unwrap().confirmed = false;
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
        self.mode_view.lock().unwrap().confirmed = false;
        self.take_pending();
        let _ = self.events.send(Event::Closed);
    }
}

/// What the UI may be told about the mode.
#[derive(Default)]
struct ModeView {
    /// The last pin holds and no disallowed report came since.
    confirmed: bool,
    /// An allowed report read while a pin was outstanding, and where it came.
    deferred: Option<(String, u64)>,
}

/// What a list of config options says about the mode.
#[derive(Debug, PartialEq, Eq)]
enum ModeReport<'a> {
    /// The list has no `mode` option, so it is about something else.
    NotReported,
    Mode(&'a str),
    /// No list, or a `mode` option without a readable value.
    Malformed,
}

fn mode_report(config_options: &Value) -> ModeReport<'_> {
    let Some(options) = config_options.as_array() else {
        return ModeReport::Malformed;
    };
    match options
        .iter()
        .find(|o| o["id"].as_str() == Some(MODE_OPTION))
    {
        None => ModeReport::NotReported,
        Some(option) => match option["currentValue"].as_str() {
            Some(mode) => ModeReport::Mode(mode),
            None => ModeReport::Malformed,
        },
    }
}
