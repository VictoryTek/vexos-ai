//! Live smoke test driver (spike, not product): drives a real agent through the
//! public `Session` API and prints what the UI would see, one JSON line each.
//!
//!   smoke --cwd DIR [--spawn | --tee DIR]
//!         [--approve SUBSTR]... [--watch FILE]... < prompts (one per line)
//!
//! Every permission request is denied unless its title contains an --approve
//! substring. --watch files are checked for existence when a request arrives.
//! --spawn uses Session::spawn; otherwise the agent is launched here behind
//! `tee`, so the raw ACP traffic lands in --tee DIR, and handed to
//! Session::connect.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::json;
use tokio::process::Command;
use vexos_ai_core::{Agent, Config, Decision, Event, Session};

struct Args {
    agent: Agent,
    cwd: PathBuf,
    spawn: bool,
    tee: Option<PathBuf>,
    approve: Vec<String>,
    watch: Vec<PathBuf>,
}

fn parse() -> Args {
    let mut args = Args {
        agent: Agent::Claude,
        cwd: PathBuf::new(),
        spawn: false,
        tee: None,
        approve: vec![],
        watch: vec![],
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cwd" => args.cwd = value().into(),
            "--spawn" => args.spawn = true,
            "--tee" => args.tee = Some(value().into()),
            "--approve" => args.approve.push(value()),
            "--watch" => args.watch.push(value().into()),
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

fn emit(value: serde_json::Value) {
    println!("{value}");
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = parse();
    let prompts: Vec<String> = std::io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let config = Config::new(&args.cwd);

    let started = if args.spawn {
        Session::spawn(args.agent, config).await
    } else {
        let dir = args.tee.clone().expect("--tee DIR or --spawn");
        let cmd = args.agent.command();
        let agent = std::iter::once(cmd.program)
            .chain(cmd.args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "tee -a '{d}/in.jsonl' | {agent} | tee -a '{d}/out.jsonl'",
            d = dir.display()
        );
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .envs(cmd.env.iter().copied())
            .current_dir(&args.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("launch agent");
        let (stdin, stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
        std::mem::forget(child); // reaped by tokio; killed with the sandbox
        Session::connect(args.agent, config, stdout, stdin).await
    };
    let (session, mut events) = match started {
        Ok(s) => s,
        Err(e) => {
            emit(json!({ "start": "error", "error": e.to_string() }));
            return;
        }
    };
    emit(json!({ "start": "ok", "agent": format!("{:?}", session.agent()) }));
    let session = Arc::new(session);

    let ui = {
        let session = session.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    Event::AgentText(_) => {}
                    Event::ToolCall(c) => {
                        emit(json!({ "tool": c.title, "kind": c.kind, "status": c.status }))
                    }
                    Event::Permission(r) => {
                        let exists: Vec<_> = args
                            .watch
                            .iter()
                            .map(|w| json!({ "file": w, "exists": w.exists() }))
                            .collect();
                        let approve = args.approve.iter().any(|a| r.title.contains(a.as_str()));
                        let decision = if approve && r.choices.contains(&Decision::ApproveOnce) {
                            Decision::ApproveOnce
                        } else {
                            Decision::DenyOnce
                        };
                        emit(
                            json!({ "permission": r.title, "kind": r.kind, "choices": format!("{:?}", r.choices),
                                     "watch_at_request": exists, "decision": format!("{decision:?}") }),
                        );
                        if let Err(e) = session.decide(r.id, decision) {
                            emit(json!({ "decide_error": e.to_string() }));
                        }
                    }
                    Event::Mode(mode) => emit(json!({ "mode": mode })),
                    Event::TimedOut(id) => emit(json!({ "timed_out": id })),
                    Event::Aborted(why) => emit(json!({ "aborted": why })),
                    Event::Closed => emit(json!({ "closed": true })),
                }
            }
        })
    };

    for (i, prompt) in prompts.iter().enumerate() {
        let result = session.prompt(prompt).await;
        emit(json!({ "prompt": i, "result": format!("{result:?}") }));
    }
    let watch: Vec<_> = parse()
        .watch
        .iter()
        .map(|w| json!({ "file": w, "exists": w.exists() }))
        .collect();
    emit(json!({ "watch_at_end": watch }));
    drop(session);
    ui.abort();
}
