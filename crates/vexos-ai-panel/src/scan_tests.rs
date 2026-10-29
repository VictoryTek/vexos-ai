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
const STYLE: &str = include_str!("style.rs");

/// The view-model's sources, where an `Outgoing` can be made.
const VIEWMODEL: [(&str, &str); 11] = [
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
    (
        "window.rs",
        include_str!("../../vexos-ai-viewmodel/src/window.rs"),
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
fn notification_code_never_sends_or_touches_a_session() {
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

/// Words that write, create, move or delete a file.
const FILE_WRITES: [&str; 10] = [
    "fs::write",
    "File::create",
    "OpenOptions",
    "create_dir",
    "remove_file",
    "remove_dir",
    "fs::rename",
    "fs::copy",
    "set_permissions",
    "write_all",
];

#[test]
fn the_panel_cannot_write_or_switch_the_agent_choice() {
    // No file is written from the panel or the view-model's shipped code.
    for (name, file) in [
        ("main.rs", MAIN),
        ("notify.rs", NOTIFY),
        ("rebuild.rs", REBUILD),
    ] {
        let code = code(file);
        for word in FILE_WRITES {
            assert!(!code.contains(word), "{word:?} in panel {name}");
        }
    }
    for (name, file) in VIEWMODEL {
        let code = shipped(file);
        for word in FILE_WRITES {
            assert!(!code.contains(word), "{word:?} in view-model {name}");
        }
    }
    // The choice is read in one place, through the view-model's reader.
    let main = code(MAIN);
    assert_eq!(main.matches("AGENT_FILE").count(), 2); // the import and one use
    assert!(main.contains("saved_agent(&glib::user_config_dir().join(AGENT_FILE))"));
    assert_eq!(main.matches("saved_agent(").count(), 1);
    // No switcher, and no agent named by the panel: it only passes on what
    // the reader returned.
    assert!(!main.contains("Agent::Claude") && !main.contains("Agent::OpenCode"));
    assert!(!main.contains("Choose an agent") && !main.contains("Start a session"));
    // The conversation page uses a Stack (empty page or list); a switcher
    // between agents would be a StackSwitcher or a dropdown.
    assert!(!main.contains("StackSwitcher") && !main.contains("DropDown"));
    // The one Stack is the conversation page: exactly the pages "empty" and
    // "list", never a page per agent.
    assert_eq!(main.matches("gtk::Stack::builder()").count(), 1);
    assert_eq!(main.matches("add_named(").count(), 2);
    assert_eq!(main.matches("Some(\"empty\")").count(), 1);
    assert_eq!(main.matches("Some(\"list\")").count(), 1);
    // The starting note (and the waiting-draft line in it) sits above the
    // input, outside the Stack, so a pending card in the list never hides it.
    assert_eq!(main.matches("chat.append(&starting_note);").count(), 1);
    assert!(!main.contains("empty.append(&starting_note)"));
    for word in ["ViewSwitcher", "ComboRow", "ToggleGroup", "ToggleButton"] {
        assert!(!main.contains(word), "{word}");
    }
    assert_eq!(main.matches("fn start(").count(), 1);
    assert_eq!(main.matches(".start(").count(), 1);
    // The reader opens read-only, and reads a bounded part.
    let (_, conversation) = VIEWMODEL[2];
    let reader = vm_body(conversation, "saved_agent");
    assert!(reader.contains("File::open(path)"));
    assert!(reader.contains(".take(MAX_CHOICE_BYTES)"));
    assert!(reader.contains("is_file()"));
}

#[test]
fn a_started_session_sends_no_text() {
    // Starting a session only spawns the agent; nothing in `start` or in the
    // window's opening touches the input, the composer or a prompt.
    let start = body("start");
    for word in [
        "composer",
        "submit",
        "send_prompt",
        "input",
        "check_usage",
        "take_launch_usage",
        ".prompt(",
        "Outgoing",
    ] {
        assert!(!start.contains(word), "{word:?} in start");
    }
    let main = code(MAIN);
    assert_eq!(main.matches("Session::spawn(").count(), 1);
    let from = main.find("fn window(").expect("window");
    let open = &main[from..from
        + main[from..]
            .find(
                "
}
",
            )
            .expect("window end")];
    for word in ["submit", "send_prompt", "composer", ".prompt("] {
        assert!(!open.contains(word), "{word:?} in window");
    }
    // The note about a waiting draft only reads whether one is waiting.
    assert!(code(MAIN).contains("c.starting_note(self.composer.borrow().waiting())"));
}

#[test]
fn launch_timing_is_display_only() {
    let main = code(MAIN);
    assert_eq!(main.matches("launch_summary()").count(), 1);
    assert!(main.contains("self.title.set_tooltip_text(c.launch_summary().as_deref());"));
    let (_, conversation) = VIEWMODEL[2];
    // main.rs has a `#[cfg(test)]` near its top, so `shipped` would cut it
    // short: scan all of its code.
    for (name, code) in [
        ("main.rs", code(MAIN)),
        ("conversation.rs", shipped(conversation)),
    ] {
        // Not a bare "log::", which is inside "AlertDialog::".
        for word in [
            "println!",
            "eprintln!",
            "dbg!",
            "log::info",
            "log::debug",
            "log::warn",
            "log::error",
            "tracing::",
            "glib::g_",
        ] {
            assert!(!code.contains(word), "{word:?} in {name}");
        }
    }
}

#[test]
fn the_window_mode_is_written_only_as_one_of_its_two_fixed_strings() {
    let main = code(MAIN);
    assert_eq!(main.matches("set_string(").count(), 1);
    assert!(main.contains("set_string(WINDOW_MODE_KEY, self.mode().toggled().as_str())"));
    // No other setting is written, and the mode is not built from text.
    for word in [
        "settings.set_value(",
        "settings.set_boolean(",
        "settings.set_int(",
        "format!(\"floating",
        "format!(\"docked",
    ] {
        assert!(!main.contains(word), "{word}");
    }
    let window = code(include_str!("../../vexos-ai-viewmodel/src/window.rs"));
    let shipped = window.split("#[cfg(test)]").next().unwrap();
    assert!(!shipped.contains("String"), "the mode carries no text");
}

#[test]
fn css_is_loaded_once_from_the_bundled_sheet_and_never_from_text() {
    let style = code(STYLE);
    let shipped = style.split("#[cfg(test)]").next().unwrap();
    assert_eq!(shipped.matches("CssProvider::new()").count(), 1);
    assert_eq!(shipped.matches("load_from_").count(), 1);
    assert!(shipped.contains("css.load_from_string(STYLESHEET);"));
    assert!(shipped.contains("pub const STYLESHEET: &str = include_str!(\"style.css\");"));
    assert_eq!(shipped.matches(".add_provider(").count(), 0);
    // The sheet's high-contrast rule needs the provider bound to GTK's setting.
    assert!(
        shipped.contains(".bind_property(\"gtk-interface-contrast\", &css, \"prefers-contrast\")")
    );
    assert!(
        shipped.find(".bind_property(").unwrap() < shipped.find("css.load_from_string").unwrap()
    );
    assert_eq!(
        shipped
            .matches("style_context_add_provider_for_display")
            .count(),
        1
    );
    // No other file makes a provider, loads a sheet, or builds markup.
    for (name, file) in [
        ("main.rs", code(MAIN)),
        ("notify.rs", code(NOTIFY)),
        ("rebuild.rs", code(REBUILD)),
    ] {
        for word in [
            "CssProvider",
            "load_from_",
            ".add_provider(",
            "style_context_add_provider",
            "set_markup",
            "use_markup",
            "from_markup",
            "markup(",
            "StatusPage",
        ] {
            assert!(!file.contains(word), "{word:?} in {name}");
        }
    }
    // The one place the sheet is loaded is the startup hook.
    assert_eq!(code(MAIN).matches("style::load(").count(), 1);
}

#[test]
fn the_title_buttons_come_only_from_the_fixed_layouts() {
    let main = code(MAIN);
    assert_eq!(main.matches("set_decoration_layout(").count(), 2);
    assert!(main.contains("set_decoration_layout(Some(docked_layout(&system)))"));
    assert!(main.contains("set_decoration_layout(None)"));
    // Docked only, and floating goes back to the system's set.
    let look = main
        .find("if look.close_only {")
        .expect("close_only branch");
    let rest = &main[look..];
    assert!(rest.find("docked_layout(").unwrap() < rest.find("} else {").unwrap());
    assert!(rest.find("set_decoration_layout(None)").unwrap() > rest.find("} else {").unwrap());
}

#[test]
fn style_classes_are_fixed_literals_in_every_panel_file() {
    for (name, file) in [
        ("main.rs", code(MAIN)),
        ("notify.rs", code(NOTIFY)),
        ("rebuild.rs", code(REBUILD)),
    ] {
        for call in file.split("add_css_class(").skip(1) {
            let call = call.trim_start();
            assert!(
                call.starts_with('"') || call.starts_with("match decision"),
                "{name}: add_css_class({call:.40}"
            );
        }
    }
}
