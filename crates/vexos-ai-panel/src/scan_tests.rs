//! The send gate is tested headless in the view-model: `Composer::submit` is
//! the only maker of an `Outgoing`, refuses Enter on an unreviewed draft, and
//! `Conversation::send` takes nothing else. What is left is the GTK wiring,
//! which can't run headless, and the core's `Session::prompt`, which takes
//! any text (the core is not changed here). These checks read the source for
//! that part. Kept in their own file so the words they look for don't
//! appear in the files they read.

const NOTIFY: &str = include_str!("notify.rs");
const REBUILD: &str = include_str!("rebuild.rs");
const MAIN: &str = include_str!("main.rs");

/// The view-model's sources, where an `Outgoing` can be made.
const VIEWMODEL: [(&str, &str); 10] = [
    (
        "lib.rs",
        include_str!("../../vexos-ai-viewmodel/src/lib.rs"),
    ),
    (
        "composer.rs",
        include_str!("../../vexos-ai-viewmodel/src/composer.rs"),
    ),
    (
        "conversation.rs",
        include_str!("../../vexos-ai-viewmodel/src/conversation.rs"),
    ),
    (
        "diagnose.rs",
        include_str!("../../vexos-ai-viewmodel/src/diagnose.rs"),
    ),
    (
        "markdown.rs",
        include_str!("../../vexos-ai-viewmodel/src/markdown.rs"),
    ),
    (
        "notify.rs",
        include_str!("../../vexos-ai-viewmodel/src/notify.rs"),
    ),
    (
        "proposal.rs",
        include_str!("../../vexos-ai-viewmodel/src/proposal.rs"),
    ),
    (
        "rebuild.rs",
        include_str!("../../vexos-ai-viewmodel/src/rebuild.rs"),
    ),
    (
        "redact.rs",
        include_str!("../../vexos-ai-viewmodel/src/redact.rs"),
    ),
    (
        "usage.rs",
        include_str!("../../vexos-ai-viewmodel/src/usage.rs"),
    ),
];

/// The file without its comment lines.
fn code(file: &str) -> String {
    file.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of `fn <name>` in main.rs, up to the end of the method.
fn body(name: &str) -> String {
    let main = code(MAIN);
    let from = main.find(&format!("fn {name}(")).expect(name);
    let rest = &main[from..];
    rest[..rest.find("\n    }\n").expect(name)].to_owned()
}

#[test]
fn notification_code_never_sends_or_starts_a_session() {
    for file in [code(NOTIFY), code(REBUILD)] {
        for word in [
            "send_prompt",
            "submit",
            "Via::",
            "Outgoing",
            ".prompt(",
            "Session",
            "session",
            ".start(",
            "conversation",
            "decide",
            ".click(",
            "usage",
            "USAGE",
        ] {
            assert!(!file.contains(word), "{word:?} in notification code");
        }
    }
    assert!(NOTIFY.contains("ui.hold_draft(draft("));
}

/// The shipped code of a view-model file: no comment lines, no test module.
fn shipped(file: &str) -> String {
    let code = code(file);
    match code.find("#[cfg(test)]") {
        Some(at) => code[..at].to_owned(),
        None => code,
    }
}

#[test]
fn an_outgoing_is_made_only_by_submit_and_the_usage_action() {
    let mut made = Vec::new();
    for (name, file) in VIEWMODEL {
        let code = shipped(file);
        for (at, _) in code.match_indices("Outgoing(") {
            if !code[..at].ends_with("pub struct ") {
                let line = code[at..].lines().next().unwrap().trim().to_owned();
                made.push((name, line));
            }
        }
    }
    assert_eq!(
        made,
        [
            (
                "composer.rs",
                "Outgoing(mem::take(&mut self.text)))".to_owned()
            ),
            (
                "conversation.rs",
                "Outgoing(USAGE_COMMAND.to_owned()))".to_owned()
            ),
        ]
    );
    let (_, composer) = VIEWMODEL[1];
    let (_, conversation) = VIEWMODEL[2];
    // The field stays crate-private: the panel can't make one.
    assert!(composer.contains("pub struct Outgoing(pub(crate) String);"));
    assert!(!code(MAIN).contains("Outgoing("));
    // The usage action's text is the constant, and it takes no text.
    assert!(conversation.contains("pub const USAGE_COMMAND: &str = \"/usage\";"));
    assert!(conversation.contains("pub fn check_usage(&self) -> Option<Outgoing> {"));
}

/// The body of `fn <name>` in a view-model file's shipped code, up to the
/// brace that closes it at its own indent (a method or a top-level fn).
fn vm_body(file: &str, name: &str) -> String {
    let code = shipped(file);
    let from = code.find(&format!("fn {name}(")).expect(name);
    let line_start = code[..from].rfind('\n').map_or(0, |i| i + 1);
    let indent = code[line_start..from].len() - code[line_start..from].trim_start().len();
    let close = format!("\n{}}}\n", " ".repeat(indent));
    let rest = &code[from..];
    rest[..rest.find(&close).expect(name)].to_owned()
}

#[test]
fn the_launch_usage_is_armed_only_by_the_ready_transition() {
    let (_, conversation) = VIEWMODEL[2];
    let code = shipped(conversation);
    // Armed in one place, the Starting -> Ready transition, and handed out
    // only through `check_usage`.
    assert_eq!(code.matches("self.launch_due = true;").count(), 1);
    let open = vm_body(conversation, "open");
    assert!(open.contains("self.state = State::Ready;"));
    assert!(open.contains("self.launch_due = true;"));
    assert!(open.contains("if self.state == State::Starting && self.started"));
    assert!(
        !code.contains("State::Starting;"),
        "nothing goes back to Starting"
    );
    let take = vm_body(conversation, "take_launch_usage");
    assert!(take.contains("if !std::mem::take(&mut self.launch_due) {"));
    assert!(take.contains("let out = self.check_usage()?;"));
    assert_eq!(code.matches("self.check_usage()").count(), 1);
    // Every way into the conversation clears it first.
    for name in ["apply", "started", "send"] {
        assert!(
            vm_body(conversation, name).contains("self.launch_due = false;"),
            "{name}"
        );
    }
}

#[test]
fn usage_is_sent_only_at_launch_and_from_its_line() {
    let main = code(MAIN);
    // The launch: taken in one place, right after each call into the
    // conversation, and sent only with a session.
    assert_eq!(main.matches("take_launch_usage()").count(), 1);
    let with = body("with_conversation");
    assert!(with.contains("(changed, c.take_launch_usage())"));
    assert!(with.contains("if let Some(out) = launch\n            && self.session().is_some()"));
    // A click: one call, from the usage line's click handler.
    assert_eq!(main.matches(".check_usage()").count(), 1);
    assert!(main.contains(
        "ui.usage.connect_clicked({\n            let ui = ui.clone();\n            move |_| ui.check_usage()\n        });"
    ));
    for name in [
        "start",
        "refresh",
        "hold_draft",
        "show_input",
        "submit",
        "show_usage",
    ] {
        let f = body(name);
        assert!(
            !f.contains(".check_usage()") && !f.contains("Conversation::check_usage"),
            "{name}"
        );
    }
    // Timers can't reach it: each timer's closure only redraws or releases
    // the input lock.
    let timers: Vec<&str> = main
        .match_indices("timeout_add")
        .map(|(at, _)| {
            let rest = &main[at..];
            &rest[..rest.find("});").expect("timer end")]
        })
        .collect();
    assert_eq!(timers.len(), 2);
    for timer in timers {
        for word in [
            "check_usage",
            "take_launch_usage",
            "with_conversation",
            "send_prompt",
            "submit",
            ".prompt(",
        ] {
            assert!(!timer.contains(word), "{word:?} in a timer: {timer}");
        }
    }
    // The separate "Check usage" button is gone: the line is the only control.
    assert!(!main.contains("\"Check usage\""));
    assert!(!main.contains("composer.append(&usage)"));
    assert!(main.contains("content.add_top_bar(&usage);"));
    assert_eq!(main.matches("ui.usage.connect_").count(), 1);
}

#[test]
fn the_usage_label_comes_only_from_the_parser() {
    let main = code(MAIN);
    // The line's text is set in one place, from the view-model's label.
    assert_eq!(main.matches("self.usage.set_label(").count(), 1);
    let show = body("show_usage");
    assert!(show.contains("let line = c.usage_button(Instant::now());"));
    assert!(show.contains("self.usage.set_label(&text);"));
    assert!(show.contains("update_property(&[gtk::accessible::Property::Label(&text)])"));
    // In the view-model, a figure exists only from the parser.
    let (_, conversation) = VIEWMODEL[2];
    let (_, usage) = VIEWMODEL[9];
    let mut known = Vec::new();
    let mut parsed = Vec::new();
    for (name, file) in VIEWMODEL {
        let code = shipped(file);
        known.extend(code.matches("Shown::Known(").map(|_| name));
        parsed.extend(code.matches("FiveHour {").map(|_| name));
    }
    assert_eq!(known, ["conversation.rs", "usage.rs"]);
    assert_eq!(parsed, ["usage.rs", "usage.rs"]);
    assert!(vm_body(conversation, "record_usage").contains("reply.and_then(five_hour)"));
    assert!(vm_body(usage, "five_hour").contains("Some(FiveHour {"));
}

#[test]
fn the_usage_click_sends_only_the_view_models_message() {
    // It sends the view-model's message as is, and never touches the input.
    let f = body("check_usage");
    assert!(f.starts_with("fn check_usage(self: &Rc<Self>)"));
    // Without a session, `send_prompt` would put the text in the input.
    assert!(f.contains("if self.session().is_none() {\n            return;\n        }"));
    assert!(f.contains(".and_then(Conversation::check_usage);"));
    assert!(f.contains("self.send_prompt(out);"));
    for word in ["composer", "input", "text", "submit", "format!"] {
        assert!(!f.contains(word), "{word:?} in check_usage");
    }
}

#[test]
fn prompts_go_out_only_through_the_composer() {
    let main = code(MAIN);
    // One prompt call, in `send_prompt`, which needs an `Outgoing` and sends
    // its text.
    assert_eq!(main.matches(".prompt(").count(), 1);
    let send = body("send_prompt");
    assert!(send.starts_with("fn send_prompt(self: &Rc<Self>, out: Outgoing)"));
    assert!(send.contains("let text = out.into_text();"));
    assert!(send.contains("session.prompt(&text)"));
    // `send_prompt` is called only by `submit`, with what the composer let
    // out, and with the view-model's `/usage`: by `check_usage` on a click,
    // and by `with_conversation` at launch.
    assert_eq!(main.matches("send_prompt(").count(), 4);
    let submit = body("submit");
    assert!(submit.contains("self.composer.borrow_mut().submit(via, can_send)"));
    assert!(submit.contains("self.send_prompt(out)"));
    assert!(body("check_usage").contains("self.send_prompt(out)"));
    assert!(body("with_conversation").contains("self.send_prompt(out)"));
    // And `submit` only by the Send button and the Enter key.
    assert_eq!(main.matches(".submit(Via::SendButton)").count(), 1);
    assert_eq!(main.matches(".submit(Via::EnterKey)").count(), 1);
    assert_eq!(main.matches("submit(").count(), 4);
    // A held draft and the answer about placing it only change the input.
    for name in ["hold_draft", "ask_placement", "show_input"] {
        let f = body(name);
        assert!(
            !f.contains("submit") && !f.contains("send_prompt") && !f.contains("prompt("),
            "{f}"
        );
    }
}
