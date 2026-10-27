//! The safety rules, driven against a fake ACP agent on an in-memory pipe.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;
use vexos_ai_core::{Agent, Config, Decision, Error, Event, Events, Session};

/// The agent side of the pipe. Records everything the core sends.
struct Fake {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
    seen: Vec<Value>,
}

impl Fake {
    /// The next message from the core, or `None` once it closed the pipe.
    /// Fails instead of hanging when the core goes quiet; the limit is above the
    /// permission timeout so paused-time tests still see the core time out first.
    async fn next(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(Duration::from_secs(60), self.lines.next_line())
            .await
            .expect("the core sent nothing for 60 s")
            .ok()??;
        let message: Value = serde_json::from_str(&line).unwrap();
        self.seen.push(message.clone());
        Some(message)
    }

    async fn expect(&mut self, method: &str) -> Value {
        let message = self
            .next()
            .await
            .unwrap_or_else(|| panic!("closed while expecting {method}"));
        assert_eq!(message["method"], method, "{message}");
        message
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
    }

    async fn reply(&mut self, to: &Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": to["id"], "result": result }))
            .await;
    }

    async fn fail(&mut self, to: &Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": to["id"], "error": { "code": -32603, "message": "nope" } }))
            .await;
    }

    async fn update(&mut self, update: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": update } }))
            .await;
    }

    /// Asks for permission with every option kind ACP has.
    async fn ask(&mut self, rpc_id: u64) {
        let options = json!([
            { "optionId": "always", "name": "Always allow", "kind": "allow_always" },
            { "optionId": "yes", "name": "Allow once", "kind": "allow_once" },
            { "optionId": "never", "name": "Always reject", "kind": "reject_always" },
            { "optionId": "no", "name": "Reject", "kind": "reject_once" },
        ]);
        let tool_call = json!({ "toolCallId": "t1", "title": "touch x", "kind": "execute", "rawInput": { "command": "touch x" } });
        self.send(json!({
            "jsonrpc": "2.0", "id": rpc_id, "method": "session/request_permission",
            "params": { "sessionId": "s1", "toolCall": tool_call, "options": options },
        }))
        .await;
    }

    /// The core's answer to permission request `rpc_id`.
    async fn answer(&mut self, rpc_id: u64) -> Value {
        let message = self.next().await.expect("closed before answering");
        assert_eq!(message["id"], rpc_id, "{message}");
        message["result"]["outcome"].clone()
    }
}

fn config() -> Config {
    // Absolute on every platform (the core refuses a relative cwd, and on
    // Windows "/tmp/..." has no drive). The fake agent never touches it.
    let mut config = Config::new(std::env::temp_dir().join("vexos-ai-test"));
    config.permission_timeout = Duration::from_secs(30);
    config
}

type Start = JoinHandle<Result<(Session, Events), Error>>;

fn begin(agent: Agent, config: Config) -> (Start, Fake) {
    let (core, agent_end) = tokio::io::duplex(1 << 16);
    let (r, w) = tokio::io::split(core);
    let start = tokio::spawn(Session::connect(agent, config, r, w));
    let (r, writer) = tokio::io::split(agent_end);
    (
        start,
        Fake {
            lines: BufReader::new(r).lines(),
            writer,
            seen: Vec::new(),
        },
    )
}

/// initialize and session/new, with the session reporting `new_result`.
async fn handshake(fake: &mut Fake, new_result: Value) {
    let init = fake.expect("initialize").await;
    fake.reply(
        &init,
        json!({ "protocolVersion": 1, "agentCapabilities": {} }),
    )
    .await;
    let new = fake.expect("session/new").await;
    fake.reply(&new, new_result).await;
}

fn claude_session(current: &str) -> Value {
    json!({ "sessionId": "s1", "modes": { "currentModeId": current, "availableModes": [
        { "id": "default" }, { "id": "acceptEdits" }, { "id": "plan" }, { "id": "auto" } ] } })
}

fn opencode_options(current: &str) -> Value {
    json!([{ "id": "mode", "category": "mode", "currentValue": current,
             "options": [{ "value": "build" }, { "value": "plan" }, { "value": "yolo" }] }])
}

/// A Claude session that is up and pinned.
async fn claude() -> (Session, Events, Fake) {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("default")).await;
    let set = fake.expect("session/set_mode").await;
    fake.reply(&set, json!({})).await;
    let (session, events) = start.await.unwrap().unwrap();
    (session, events, fake)
}

/// An OpenCode session that is up and pinned.
async fn opencode() -> (Session, Events, Fake) {
    let (start, mut fake) = begin(Agent::OpenCode, config());
    handshake(
        &mut fake,
        json!({ "sessionId": "s1", "configOptions": opencode_options("build") }),
    )
    .await;
    let set = fake.expect("session/set_config_option").await;
    fake.reply(&set, json!({ "configOptions": opencode_options("build") }))
        .await;
    let (session, events) = start.await.unwrap().unwrap();
    (session, events, fake)
}

/// Claude's mode as the adapter also reports it: a `mode` config option.
fn claude_mode_option(current: &Value) -> Value {
    json!([{ "id": "mode", "category": "mode", "currentValue": current },
           { "id": "model", "category": "model", "currentValue": "opus" }])
}

/// The prompt goes straight out: no reset happened first.
async fn prompt_goes_through(session: &Arc<Session>, fake: &mut Fake) {
    let s = session.clone();
    let prompt = tokio::spawn(async move { s.prompt("hi").await });
    let p = fake.expect("session/prompt").await;
    fake.reply(&p, json!({ "stopReason": "end_turn" })).await;
    assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
}

async fn next_permission(events: &mut Events) -> vexos_ai_core::PermissionRequest {
    loop {
        match events.recv().await.expect("events ended") {
            Event::Permission(request) => return request,
            _ => continue,
        }
    }
}

// 1. Mode pinning --------------------------------------------------------------

#[tokio::test]
async fn claude_is_reset_to_default_even_when_settings_started_it_in_accept_edits() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("acceptEdits")).await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(
        set["params"],
        json!({ "sessionId": "s1", "modeId": "default" })
    );
    fake.reply(&set, json!({})).await;
    let (session, _events) = start.await.unwrap().unwrap();

    let prompt = tokio::spawn(async move { session.prompt("hi").await });
    let p = fake.expect("session/prompt").await;
    fake.reply(&p, json!({ "stopReason": "end_turn" })).await;
    assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
}

#[tokio::test]
async fn claude_is_reset_even_when_it_already_reports_default() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("default")).await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(set["params"]["modeId"], "default");
    fake.reply(&set, json!({})).await;
    start.await.unwrap().unwrap();
}

#[tokio::test]
async fn opencode_is_reset_to_build_even_when_a_project_made_a_custom_agent_the_default() {
    let (start, mut fake) = begin(Agent::OpenCode, config());
    handshake(
        &mut fake,
        json!({ "sessionId": "s1", "configOptions": opencode_options("yolo") }),
    )
    .await;
    let set = fake.expect("session/set_config_option").await;
    assert_eq!(
        set["params"],
        json!({ "sessionId": "s1", "configId": "mode", "value": "build" })
    );
    fake.reply(&set, json!({ "configOptions": opencode_options("build") }))
        .await;
    start.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_failed_claude_reset_aborts_the_session() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("acceptEdits")).await;
    let set = fake.expect("session/set_mode").await;
    fake.fail(&set).await;
    assert!(matches!(start.await.unwrap(), Err(Error::ModePin(_))));
    assert!(
        fake.next().await.is_none(),
        "the core must close the connection"
    );
}

#[tokio::test]
async fn an_opencode_reset_that_does_not_take_aborts_the_session() {
    let (start, mut fake) = begin(Agent::OpenCode, config());
    handshake(
        &mut fake,
        json!({ "sessionId": "s1", "configOptions": opencode_options("yolo") }),
    )
    .await;
    let set = fake.expect("session/set_config_option").await;
    fake.reply(&set, json!({ "configOptions": opencode_options("yolo") }))
        .await;
    assert!(matches!(start.await.unwrap(), Err(Error::ModePin(_))));
    assert!(fake.next().await.is_none());
}

#[tokio::test]
async fn a_mid_session_mode_change_is_reset() {
    let (session, _events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "acceptEdits" }))
        .await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(set["params"]["modeId"], "default");
    fake.reply(&set, json!({})).await;

    // The prompt waits for nothing: once reset, the session works again.
    let prompt = tokio::spawn(async move {
        loop {
            match session.prompt("hi").await {
                Err(Error::NotPinned) => tokio::task::yield_now().await,
                other => return other,
            }
        }
    });
    let p = fake.expect("session/prompt").await;
    fake.reply(&p, json!({ "stopReason": "end_turn" })).await;
    assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
}

#[tokio::test]
async fn a_failed_mid_session_reset_aborts_and_refuses_prompts() {
    let (session, mut events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "acceptEdits" }))
        .await;
    let set = fake.expect("session/set_mode").await;
    fake.fail(&set).await;
    loop {
        match events.recv().await.expect("events ended") {
            Event::Aborted(reason) => {
                assert!(reason.contains("mode reset failed"), "{reason}");
                break;
            }
            _ => continue,
        }
    }
    assert!(matches!(session.prompt("hi").await, Err(Error::Aborted)));
    assert!(fake.next().await.is_none());
}

#[tokio::test]
async fn opencode_switching_to_a_custom_agent_mid_session_is_reset() {
    let (start, mut fake) = begin(Agent::OpenCode, config());
    handshake(
        &mut fake,
        json!({ "sessionId": "s1", "configOptions": opencode_options("build") }),
    )
    .await;
    let set = fake.expect("session/set_config_option").await;
    fake.reply(&set, json!({ "configOptions": opencode_options("build") }))
        .await;
    let (_session, _events) = start.await.unwrap().unwrap();

    fake.update(json!({ "sessionUpdate": "config_option_update", "configOptions": opencode_options("yolo") })).await;
    let set = fake.expect("session/set_config_option").await;
    assert_eq!(set["params"]["value"], "build");
}

// 1b. Mode reports and the allow-list -----------------------------------------

#[tokio::test]
async fn claude_reporting_a_mode_through_the_config_option_is_reset() {
    let (_session, _events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": claude_mode_option(&json!("acceptEdits")) }))
        .await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(set["params"]["modeId"], "default");
}

#[tokio::test]
async fn opencode_reporting_a_mode_through_current_mode_update_is_reset() {
    let (_session, _events, mut fake) = opencode().await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "yolo" }))
        .await;
    let set = fake.expect("session/set_config_option").await;
    assert_eq!(set["params"]["value"], "build");
}

#[tokio::test]
async fn unreadable_mode_reports_fail_closed() {
    let (_session, _events, mut fake) = claude().await;
    for report in [
        json!({ "sessionUpdate": "current_mode_update" }),
        json!({ "sessionUpdate": "current_mode_update", "currentModeId": 7 }),
        json!({ "sessionUpdate": "config_option_update" }),
        json!({ "sessionUpdate": "config_option_update", "configOptions": "mode" }),
        json!({ "sessionUpdate": "config_option_update", "configOptions": claude_mode_option(&json!(null)) }),
    ] {
        fake.update(report.clone()).await;
        let set = fake.expect("session/set_mode").await;
        assert_eq!(set["params"]["modeId"], "default", "after {report}");
        fake.reply(&set, json!({})).await;
    }
}

#[tokio::test]
async fn config_updates_that_are_not_about_the_mode_are_ignored() {
    let (session, _events, mut fake) = claude().await;
    let session = Arc::new(session);
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": [{ "id": "model", "currentValue": "haiku" }] }))
        .await;
    prompt_goes_through(&session, &mut fake).await;
}

#[tokio::test]
async fn claude_may_be_in_plan_mode() {
    let (session, _events, mut fake) = claude().await;
    let session = Arc::new(session);
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "plan" }))
        .await;
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": claude_mode_option(&json!("plan")) }))
        .await;
    prompt_goes_through(&session, &mut fake).await;
}

#[tokio::test]
async fn leaving_plan_for_auto_resets_to_default_not_plan() {
    let (_session, _events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "plan" }))
        .await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "auto" }))
        .await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(set["params"]["modeId"], "default");
}

#[tokio::test]
async fn opencode_plan_is_not_allowed() {
    let (_session, _events, mut fake) = opencode().await;
    fake.update(json!({ "sessionUpdate": "config_option_update", "configOptions": opencode_options("plan") }))
        .await;
    let set = fake.expect("session/set_config_option").await;
    assert_eq!(set["params"]["value"], "build");
}

#[tokio::test]
async fn a_session_reported_in_plan_still_starts_in_default() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("plan")).await;
    let set = fake.expect("session/set_mode").await;
    assert_eq!(set["params"]["modeId"], "default");
    fake.reply(&set, json!({})).await;
    start.await.unwrap().unwrap();
}

#[tokio::test]
async fn prompts_are_refused_until_the_reset_is_acknowledged() {
    let (session, _events, mut fake) = claude().await;
    let session = Arc::new(session);
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": claude_mode_option(&json!("acceptEdits")) }))
        .await;
    let set = fake.expect("session/set_mode").await;
    assert_refused(&session).await;
    fake.reply(&set, json!({})).await;
    loop {
        // The reply is handled on another task; wait for it to land.
        let s = session.clone();
        let prompt = tokio::spawn(async move { s.prompt("hi").await });
        tokio::task::yield_now().await;
        if prompt.is_finished() {
            assert!(matches!(prompt.await.unwrap(), Err(Error::NotPinned)));
            continue;
        }
        let p = fake.expect("session/prompt").await;
        fake.reply(&p, json!({ "stopReason": "end_turn" })).await;
        assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
        break;
    }
}

/// The startup race: a disallowed report read right after the first reset's
/// acknowledgement, before the session is handed out. The session must start
/// refusing prompts until that report's own reset is acknowledged.
#[tokio::test]
async fn a_report_right_after_the_first_pin_keeps_prompts_refused_until_its_reset() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("default")).await;
    let first = fake.expect("session/set_mode").await;
    // Both lines are read before the start task resumes.
    fake.reply(&first, json!({})).await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "acceptEdits" }))
        .await;
    let second = fake.expect("session/set_mode").await;
    assert_eq!(second["params"]["modeId"], "default");
    let (session, _events) = start.await.unwrap().unwrap();
    let session = Arc::new(session);
    assert_refused(&session).await;

    fake.reply(&second, json!({})).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    prompt_goes_through(&session, &mut fake).await;
}

#[tokio::test]
async fn an_older_reset_does_not_reopen_prompts_while_a_newer_one_runs() {
    let (session, _events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "acceptEdits" }))
        .await;
    let first = fake.expect("session/set_mode").await;
    fake.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "auto" }))
        .await;
    let second = fake.expect("session/set_mode").await;
    fake.reply(&first, json!({})).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_refused(&Arc::new(session)).await;
    fake.reply(&second, json!({})).await;
}

/// The prompt is refused without being sent. A prompt that was sent would wait
/// for an answer that never comes, so it fails on the timeout instead.
async fn assert_refused(session: &Arc<Session>) {
    let result = tokio::time::timeout(Duration::from_secs(5), session.prompt("hi")).await;
    assert!(matches!(result, Ok(Err(Error::NotPinned))), "{result:?}");
}

#[tokio::test]
async fn a_failed_reset_after_a_config_option_report_aborts() {
    let (session, mut events, mut fake) = claude().await;
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": claude_mode_option(&json!("bypassPermissions")) }))
        .await;
    let set = fake.expect("session/set_mode").await;
    fake.fail(&set).await;
    loop {
        if let Event::Aborted(_) = events.recv().await.expect("events ended") {
            break;
        }
    }
    assert!(matches!(session.prompt("hi").await, Err(Error::Aborted)));
}

// 1c. The mode shown to the UI ------------------------------------------------

/// The modes the core has shown since the last call. A permission request is
/// the barrier: the core reads the agent's messages in order, so everything it
/// showed for earlier messages is queued before that request reaches the UI.
async fn modes_shown(fake: &mut Fake, events: &mut Events, barrier: u64) -> Vec<String> {
    fake.ask(barrier).await;
    let mut modes = Vec::new();
    loop {
        match events.recv().await.expect("events ended") {
            Event::Mode(mode) => modes.push(mode),
            Event::Permission(_) => return modes,
            _ => continue,
        }
    }
}

fn mode_update(mode: &str) -> Value {
    json!({ "sessionUpdate": "current_mode_update", "currentModeId": mode })
}

#[tokio::test]
async fn the_start_mode_is_shown_once_pinned() {
    let (_session, mut events, mut fake) = claude().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["default"]);
    let (_session, mut events, mut fake) = opencode().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["build"]);
}

#[tokio::test]
async fn allowed_reports_are_shown() {
    let (_session, mut events, mut fake) = claude().await;
    fake.update(mode_update("plan")).await;
    fake.update(json!({ "sessionUpdate": "config_option_update",
                        "configOptions": claude_mode_option(&json!("default")) }))
        .await;
    assert_eq!(
        modes_shown(&mut fake, &mut events, 100).await,
        ["default", "plan", "default"]
    );
}

#[tokio::test]
async fn disallowed_and_unreadable_reports_are_never_shown() {
    let (_session, mut events, mut fake) = claude().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["default"]);
    for report in [
        mode_update("acceptEdits"),
        json!({ "sessionUpdate": "current_mode_update" }),
        json!({ "sessionUpdate": "config_option_update", "configOptions": claude_mode_option(&json!("auto")) }),
    ] {
        fake.update(report).await;
        let _unanswered = fake.expect("session/set_mode").await;
    }
    // An allowed report while those resets run waits for them.
    fake.update(mode_update("plan")).await;
    assert!(modes_shown(&mut fake, &mut events, 101).await.is_empty());
}

#[tokio::test]
async fn an_acknowledged_reset_shows_the_start_mode() {
    let (_session, mut events, mut fake) = opencode().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["build"]);
    fake.update(json!({ "sessionUpdate": "config_option_update", "configOptions": opencode_options("yolo") }))
        .await;
    let set = fake.expect("session/set_config_option").await;
    assert!(modes_shown(&mut fake, &mut events, 101).await.is_empty());
    fake.reply(&set, json!({ "configOptions": opencode_options("build") }))
        .await;
    assert_eq!(modes_shown(&mut fake, &mut events, 102).await, ["build"]);
}

#[tokio::test]
async fn a_report_from_before_the_reset_took_is_not_shown() {
    let (_session, mut events, mut fake) = claude().await;
    fake.update(mode_update("acceptEdits")).await;
    let set = fake.expect("session/set_mode").await;
    fake.update(mode_update("plan")).await;
    fake.reply(&set, json!({})).await;
    assert_eq!(
        modes_shown(&mut fake, &mut events, 100).await,
        ["default", "default"]
    );
}

#[tokio::test]
async fn a_report_from_after_the_reset_took_is_shown_after_it() {
    let (_session, mut events, mut fake) = claude().await;
    fake.update(mode_update("acceptEdits")).await;
    let set = fake.expect("session/set_mode").await;
    // Both lines are read before the reset's task wakes up.
    fake.reply(&set, json!({})).await;
    fake.update(mode_update("plan")).await;
    assert_eq!(
        modes_shown(&mut fake, &mut events, 100).await,
        ["default", "default", "plan"]
    );
}

#[tokio::test]
async fn an_older_reset_shows_nothing_while_a_newer_one_runs() {
    let (_session, mut events, mut fake) = claude().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["default"]);
    fake.update(mode_update("acceptEdits")).await;
    let first = fake.expect("session/set_mode").await;
    fake.update(mode_update("auto")).await;
    let second = fake.expect("session/set_mode").await;
    fake.reply(&first, json!({})).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(modes_shown(&mut fake, &mut events, 101).await.is_empty());
    fake.reply(&second, json!({})).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(modes_shown(&mut fake, &mut events, 102).await, ["default"]);
}

#[tokio::test]
async fn a_disallowed_report_during_the_first_pin_leaves_the_mode_to_its_reset() {
    let (start, mut fake) = begin(Agent::Claude, config());
    handshake(&mut fake, claude_session("default")).await;
    let first = fake.expect("session/set_mode").await;
    fake.update(mode_update("acceptEdits")).await;
    let second = fake.expect("session/set_mode").await;
    fake.reply(&first, json!({})).await;
    let (_session, mut events) = start.await.unwrap().unwrap();
    assert!(modes_shown(&mut fake, &mut events, 100).await.is_empty());
    fake.reply(&second, json!({})).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(modes_shown(&mut fake, &mut events, 101).await, ["default"]);
}

#[tokio::test]
async fn a_failed_reset_shows_no_mode() {
    let (_session, mut events, mut fake) = claude().await;
    assert_eq!(modes_shown(&mut fake, &mut events, 100).await, ["default"]);
    fake.update(mode_update("bypassPermissions")).await;
    let set = fake.expect("session/set_mode").await;
    fake.fail(&set).await;
    loop {
        match events.recv().await.expect("events ended") {
            Event::Mode(mode) => panic!("showed {mode}"),
            Event::Aborted(_) => break,
            _ => continue,
        }
    }
}

// 2. Options filter -------------------------------------------------------------

/// The options claude-agent-acp 0.88.0 offered on plan exit, recorded live
/// (`assistant_panel_spike_results.md`, "Core live smoke test", check 4).
#[tokio::test]
async fn plan_exit_keeps_exit_to_default_and_keep_planning_only() {
    let (session, mut events, mut fake) = claude().await;
    let options = json!([
        { "kind": "allow_once", "name": "Yes, manually approve edits", "optionId": "exit-plan-default" },
        { "kind": "allow_always", "name": "Yes, clear context (4% used) and use auto mode", "optionId": "exit-plan-clear-auto" },
        { "kind": "allow_always", "name": "Yes, and use auto mode", "optionId": "exit-plan-auto" },
        { "kind": "reject_once", "name": "No, keep planning", "optionId": "reject" },
    ]);
    for (rpc_id, decision, expected) in [
        (20, Decision::ApproveOnce, "exit-plan-default"),
        (21, Decision::DenyOnce, "reject"),
    ] {
        fake.send(json!({
            "jsonrpc": "2.0", "id": rpc_id, "method": "session/request_permission",
            "params": { "sessionId": "s1", "options": options,
                        "toolCall": { "toolCallId": "t9", "title": "Approve Plan", "kind": "switch_mode" } },
        }))
        .await;
        let request = next_permission(&mut events).await;
        assert_eq!(request.choices, [Decision::ApproveOnce, Decision::DenyOnce]);
        session.decide(request.id, decision).unwrap();
        assert_eq!(
            fake.answer(rpc_id).await,
            json!({ "outcome": "selected", "optionId": expected })
        );
    }
}

#[tokio::test]
async fn always_options_never_reach_the_ui_and_approve_means_once() {
    let (session, mut events, mut fake) = claude().await;
    fake.ask(7).await;
    let request = next_permission(&mut events).await;
    assert_eq!(request.choices, [Decision::ApproveOnce, Decision::DenyOnce]);
    assert_eq!(request.title, "touch x");

    session.decide(request.id, Decision::ApproveOnce).unwrap();
    assert_eq!(
        fake.answer(7).await,
        json!({ "outcome": "selected", "optionId": "yes" })
    );
}

#[tokio::test]
async fn a_decision_that_was_not_offered_is_refused() {
    let (session, mut events, mut fake) = claude().await;
    fake.send(json!({
        "jsonrpc": "2.0", "id": 8, "method": "session/request_permission",
        "params": { "sessionId": "s1", "toolCall": { "toolCallId": "t1", "title": "x" },
                    "options": [{ "optionId": "always", "kind": "allow_always" }, { "optionId": "no", "kind": "reject_once" }] },
    }))
    .await;
    let request = next_permission(&mut events).await;
    assert_eq!(request.choices, [Decision::DenyOnce]);
    assert!(matches!(
        session.decide(request.id, Decision::ApproveOnce),
        Err(Error::InvalidDecision)
    ));
    session.decide(request.id, Decision::DenyOnce).unwrap();
    assert_eq!(
        fake.answer(8).await,
        json!({ "outcome": "selected", "optionId": "no" })
    );
}

// 3. Never auto-approve ---------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn an_unanswered_request_is_denied_on_timeout() {
    let (_session, mut events, mut fake) = claude().await;
    fake.ask(9).await;
    let _request = next_permission(&mut events).await;
    // Paused time jumps to the 30 s timeout once nothing else can run.
    assert_eq!(
        fake.answer(9).await,
        json!({ "outcome": "selected", "optionId": "no" })
    );
}

/// The next event other than mode and tool updates, or `None` if nothing comes
/// within twice the permission timeout.
async fn next_answer_event(events: &mut Events) -> Option<Event> {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Some(Event::Mode(_) | Event::ToolCall(_)) => continue,
                other => return other,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_request_is_reported_after_it_was_denied() {
    let (_session, mut events, mut fake) = claude().await;
    fake.ask(9).await;
    let request = next_permission(&mut events).await;
    assert_eq!(
        fake.answer(9).await,
        json!({ "outcome": "selected", "optionId": "no" })
    );
    assert!(matches!(
        next_answer_event(&mut events).await,
        Some(Event::TimedOut(id)) if id == request.id
    ));
}

#[tokio::test(start_paused = true)]
async fn answered_or_cancelled_requests_are_never_reported_timed_out() {
    let (session, mut events, mut fake) = claude().await;
    // Checked after each answer: `next_permission` skips other events.
    for (rpc_id, decision, option) in [
        (9, Decision::ApproveOnce, "yes"),
        (10, Decision::DenyOnce, "no"),
    ] {
        fake.ask(rpc_id).await;
        let request = next_permission(&mut events).await;
        session.decide(request.id, decision).unwrap();
        assert_eq!(
            fake.answer(rpc_id).await,
            json!({ "outcome": "selected", "optionId": option })
        );
        let stray = next_answer_event(&mut events).await;
        assert!(stray.is_none(), "{decision:?}: {stray:?}");
    }
    fake.ask(11).await;
    next_permission(&mut events).await;
    session.cancel().await;
    assert_eq!(fake.answer(11).await, json!({ "outcome": "cancelled" }));
    assert!(next_answer_event(&mut events).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_timeout_with_no_deny_option_cancels_rather_than_approves() {
    let (_session, mut events, mut fake) = claude().await;
    fake.send(json!({
        "jsonrpc": "2.0", "id": 10, "method": "session/request_permission",
        "params": { "sessionId": "s1", "toolCall": { "toolCallId": "t1", "title": "x" },
                    "options": [{ "optionId": "yes", "kind": "allow_once" }] },
    }))
    .await;
    let _request = next_permission(&mut events).await;
    assert_eq!(fake.answer(10).await, json!({ "outcome": "cancelled" }));
}

#[tokio::test]
async fn with_nobody_listening_a_request_is_denied_at_once() {
    let (_session, events, mut fake) = claude().await;
    drop(events);
    fake.ask(11).await;
    assert_eq!(
        fake.answer(11).await,
        json!({ "outcome": "selected", "optionId": "no" })
    );
}

#[tokio::test]
async fn dropping_the_session_denies_what_is_pending_and_disconnects() {
    let (session, mut events, mut fake) = claude().await;
    fake.ask(12).await;
    let _request = next_permission(&mut events).await;
    drop(session);
    assert_eq!(
        fake.answer(12).await,
        json!({ "outcome": "selected", "optionId": "no" })
    );
    assert!(fake.next().await.is_none());
}

#[tokio::test]
async fn cancel_answers_pending_requests_cancelled() {
    let (session, mut events, mut fake) = claude().await;
    fake.ask(13).await;
    let request = next_permission(&mut events).await;
    session.cancel().await;
    assert_eq!(fake.answer(13).await, json!({ "outcome": "cancelled" }));
    fake.expect("session/cancel").await;
    assert!(matches!(
        session.decide(request.id, Decision::ApproveOnce),
        Err(Error::UnknownRequest)
    ));
}

// 4. No auto-approving mode, flag or environment ---------------------------------

#[test]
fn launch_commands_carry_no_approval_overrides() {
    let claude = Agent::Claude.command();
    assert_eq!(claude.program, "claude-agent-acp");
    assert!(claude.args.is_empty());
    assert_eq!(claude.env, [("CLAUDE_AGENT_ACP_PROTOCOL", "v1")]);

    let opencode = Agent::OpenCode.command();
    assert_eq!(opencode.program, "opencode");
    assert_eq!(opencode.args, ["acp"]);
    assert!(opencode.env.is_empty());
}

#[tokio::test]
async fn over_a_whole_session_the_core_only_sends_safe_requests() {
    let (session, mut events, mut fake) = claude().await;
    fake.ask(14).await;
    let request = next_permission(&mut events).await;
    session.decide(request.id, Decision::ApproveOnce).unwrap();
    fake.answer(14).await;
    let prompt = tokio::spawn(async move {
        let r = session.prompt("hi").await;
        session.cancel().await;
        r
    });
    let p = fake.expect("session/prompt").await;
    fake.reply(&p, json!({ "stopReason": "end_turn" })).await;
    prompt.await.unwrap().unwrap();
    fake.expect("session/cancel").await;
    assert!(fake.next().await.is_none());

    let init = &fake.seen[0];
    assert_eq!(
        init["params"]["clientCapabilities"],
        json!({ "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false })
    );
    for message in &fake.seen {
        let params = &message["params"];
        assert!(params.get("_meta").is_none(), "{message}");
        match message["method"].as_str() {
            Some("initialize" | "session/new" | "session/prompt" | "session/cancel") => {}
            Some("session/set_mode") => assert_eq!(params["modeId"], "default"),
            Some(other) => panic!("unexpected request {other}"),
            // Responses: only the one approval, and it is "once".
            None => assert_eq!(message["result"]["outcome"]["optionId"], "yes"),
        }
    }
}

#[tokio::test]
async fn requests_for_capabilities_never_advertised_are_refused() {
    let (_session, _events, mut fake) = claude().await;
    fake.send(json!({ "jsonrpc": "2.0", "id": 15, "method": "fs/write_text_file", "params": { "path": "/x", "content": "" } }))
        .await;
    let reply = fake.next().await.unwrap();
    assert_eq!(reply["id"], 15);
    assert_eq!(reply["error"]["code"], -32601);
}

#[tokio::test]
async fn a_relative_working_directory_is_refused() {
    let (core, _agent) = tokio::io::duplex(64);
    let (r, w) = tokio::io::split(core);
    let result = Session::connect(Agent::Claude, Config::new("relative"), r, w).await;
    assert!(matches!(result, Err(Error::Config(_))));
}
