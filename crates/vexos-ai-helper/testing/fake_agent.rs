//! Test-only: a fake ACP agent on an in-memory pipe, and a `Starter` that
//! connects the core to it. The agent answers the handshake, holds each prompt
//! until told to end the turn, and records everything the core sends.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use vexos_ai_core::{Agent, Config, Error, Session};
use vexos_ai_helper::service::{Started, Starter};

pub enum Cmd {
    /// A permission request, as the agent asks for one.
    Ask { rpc_id: u64, command: String },
    /// Ends the oldest running turn.
    EndTurn,
    /// The agent reports a mode the core must reset, and holds its answer to
    /// the reset until [`Cmd::AckReset`].
    DriftMode,
    /// Answers the held mode reset.
    AckReset,
}

/// Everything the core sent, in order.
#[derive(Clone, Default)]
pub struct Seen(Arc<Mutex<Vec<Value>>>);

impl Seen {
    fn push(&self, message: Value) {
        self.0.lock().unwrap().push(message);
    }

    pub fn all(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    pub fn count(&self, method: &str) -> usize {
        self.all().iter().filter(|m| m["method"] == method).count()
    }

    /// The text of each `session/prompt`.
    pub fn prompts(&self) -> Vec<String> {
        self.all()
            .iter()
            .filter(|m| m["method"] == "session/prompt")
            .map(|m| {
                m["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    /// The core's answer to permission request `rpc_id`.
    pub fn answer(&self, rpc_id: u64) -> Option<Value> {
        self.all()
            .iter()
            .find(|m| m["id"] == rpc_id && m.get("method").is_none())
            .map(|m| m["result"]["outcome"].clone())
    }
}

#[derive(Clone)]
pub struct FakeStarter {
    runtime: Handle,
    pub seen: Seen,
    pub starts: Arc<AtomicUsize>,
    /// Set when the agent's end of the pipe has closed.
    stopped: Arc<AtomicBool>,
    commands: Arc<Mutex<Option<mpsc::UnboundedSender<Cmd>>>>,
    /// Every start fails.
    pub fail: bool,
    /// How long a start takes before the session arrives.
    pub delay: Duration,
}

impl FakeStarter {
    pub fn new(runtime: Handle) -> FakeStarter {
        FakeStarter {
            runtime,
            seen: Seen::default(),
            starts: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            commands: Arc::new(Mutex::new(None)),
            fail: false,
            delay: Duration::ZERO,
        }
    }

    /// Tells the agent of the latest session what to do.
    pub fn tell(&self, cmd: Cmd) {
        let commands = self.commands.lock().unwrap();
        commands
            .as_ref()
            .expect("no session has been started")
            .send(cmd)
            .expect("the fake agent has stopped");
    }

    /// The core has closed its end of the pipe.
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

impl Starter for FakeStarter {
    fn start(&self, agent: Agent) -> Pin<Box<dyn Future<Output = Started>>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Box::pin(async { Err(Error::Closed) });
        }
        let (core, agent_end) = tokio::io::duplex(1 << 16);
        let (tx, rx) = mpsc::unbounded_channel();
        *self.commands.lock().unwrap() = Some(tx);
        self.runtime.spawn(run_agent(
            agent_end,
            self.seen.clone(),
            rx,
            self.stopped.clone(),
        ));
        let (reader, writer) = tokio::io::split(core);
        let config = Config::new(std::env::temp_dir().join("vexos-ai-test"));
        let delay = self.delay;
        let starting = self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            Session::connect(agent, config, reader, writer).await
        });
        Box::pin(async move {
            starting
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())))
        })
    }
}

async fn send(writer: &mut (impl AsyncWriteExt + Unpin), message: Value) {
    let mut line = message.to_string();
    line.push('\n');
    let _ = writer.write_all(line.as_bytes()).await;
}

async fn reply(writer: &mut (impl AsyncWriteExt + Unpin), to: &Value, result: Value) {
    send(
        writer,
        json!({ "jsonrpc": "2.0", "id": to["id"], "result": result }),
    )
    .await;
}

async fn run_agent(
    pipe: DuplexStream,
    seen: Seen,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
    stopped: Arc<AtomicBool>,
) {
    run_agent_loop(pipe, seen, &mut commands).await;
    stopped.store(true, Ordering::SeqCst);
}

async fn run_agent_loop(
    pipe: DuplexStream,
    seen: Seen,
    commands: &mut mpsc::UnboundedReceiver<Cmd>,
) {
    let (reader, mut writer) = tokio::io::split(pipe);
    let mut lines = BufReader::new(reader).lines();
    let mut turns: Vec<Value> = Vec::new();
    let mut hold_reset = false;
    let mut resets: Vec<Value> = Vec::new();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let message: Value = serde_json::from_str(&line).unwrap();
                seen.push(message.clone());
                match message["method"].as_str() {
                    Some("initialize") => {
                        reply(&mut writer, &message,
                            json!({ "protocolVersion": 1, "agentCapabilities": {} })).await
                    }
                    Some("session/new") => {
                        reply(&mut writer, &message, json!({
                            "sessionId": "s1",
                            "modes": { "currentModeId": "default", "availableModes": [
                                { "id": "default" }, { "id": "plan" }] } })).await
                    }
                    Some("session/set_mode") if hold_reset => resets.push(message),
                    Some("session/set_mode") => reply(&mut writer, &message, json!({})).await,
                    Some("session/prompt") => turns.push(message),
                    Some("session/cancel") => {
                        for turn in turns.drain(..) {
                            reply(&mut writer, &turn, json!({ "stopReason": "cancelled" })).await;
                        }
                    }
                    _ => {}
                }
            }
            Some(cmd) = commands.recv() => match cmd {
                Cmd::Ask { rpc_id, command } => {
                    let options = json!([
                        { "optionId": "always", "name": "Always allow", "kind": "allow_always" },
                        { "optionId": "yes", "name": "Allow once", "kind": "allow_once" },
                        { "optionId": "never", "name": "Always reject", "kind": "reject_always" },
                        { "optionId": "no", "name": "Reject", "kind": "reject_once" },
                    ]);
                    let tool_call = json!({ "toolCallId": "t1", "title": "run a command",
                        "kind": "execute", "rawInput": { "command": command } });
                    send(&mut writer, json!({
                        "jsonrpc": "2.0", "id": rpc_id, "method": "session/request_permission",
                        "params": { "sessionId": "s1", "toolCall": tool_call, "options": options },
                    })).await;
                }
                Cmd::DriftMode => {
                    hold_reset = true;
                    send(&mut writer, json!({
                        "jsonrpc": "2.0", "method": "session/update",
                        "params": { "sessionId": "s1", "update": {
                            "sessionUpdate": "current_mode_update",
                            "currentModeId": "acceptEdits" } },
                    })).await;
                }
                Cmd::AckReset => {
                    hold_reset = false;
                    for reset in resets.drain(..) {
                        reply(&mut writer, &reset, json!({})).await;
                    }
                }
                Cmd::EndTurn => {
                    if !turns.is_empty() {
                        let turn = turns.remove(0);
                        reply(&mut writer, &turn, json!({ "stopReason": "end_turn" })).await;
                    }
                }
            },
        }
    }
}
