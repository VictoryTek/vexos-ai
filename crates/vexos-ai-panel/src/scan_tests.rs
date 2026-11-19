//! The send gate is tested headless in the view-model: `Composer::submit` is
//! the only maker of an `Outgoing`, refuses Enter on an unreviewed draft, and
//! `Conversation::send` takes nothing else. What is left is the GTK wiring,
//! which can't run headless, and the core's `Session::prompt`, which takes
//! any text (the core is not changed here). These checks read the source for
//! that part. Kept in their own file so the words they look for don't
//! appear in the files they read.

const NOTIFY: &str = include_str!("notify.rs");
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
    (
        "starters.rs",
        include_str!("../../vexos-ai-viewmodel/src/starters.rs"),
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
    for file in [code(NOTIFY)] {
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
fn the_panel_has_no_usage_and_never_sends_the_usage_command() {
    // Plan limits live in the top-bar panel. This window neither asks for them
    // nor shows them, so nothing in it can send `/usage`.
    let main = code(MAIN);
    for word in [
        "check_usage",
        "take_launch_usage",
        "usage_button",
        "release_usage_lock",
        "usage_locked",
        "usage_disabled",
        "USAGE",
        "five_hour",
        "PanelSettings",
        "PANEL_SETTINGS_FILE",
        "lock_reveal",
        "vexos-usage",
    ] {
        assert!(!main.contains(word), "{word:?} in main.rs");
    }
    // The conversation is built with the launch `/usage` switched off.
    assert_eq!(main.matches(".usage_on_launch(").count(), 1);
    assert!(main.contains(".usage_on_launch(false)"));
    // The two timers: the agent's start deadline and the hidden-too-long limit.
    let timers: Vec<&str> = main
        .match_indices("timeout_add")
        .map(|(at, _)| {
            let rest = &main[at..];
            &rest[..rest.find("});").expect("timer end")]
        })
        .collect();
    assert_eq!(timers.len(), 2);
    assert_eq!(
        timers
            .iter()
            .filter(|t| t.contains("START_DEADLINE"))
            .count(),
        1
    );
    assert_eq!(
        timers.iter().filter(|t| t.contains("HIDDEN_LIMIT")).count(),
        1
    );
    for timer in timers {
        for word in [
            "check_usage",
            "with_conversation",
            "send_prompt",
            "submit",
            ".prompt(",
        ] {
            assert!(!timer.contains(word), "{word:?} in a timer: {timer}");
        }
    }
    assert!(
        !include_str!("style.css").contains("usage"),
        "usage in style.css"
    );
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
    // `send_prompt` is called only by `submit`, with what the composer let out.
    assert_eq!(main.matches("send_prompt(").count(), 2);
    let submit = body("submit");
    assert!(submit.contains("self.composer.borrow_mut().submit(via, can_send)"));
    assert!(submit.contains("self.send_prompt(out)"));
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
    for (name, file) in [("main.rs", MAIN), ("notify.rs", NOTIFY)] {
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
    assert!(!main.contains("Agent::Claude"));
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
    for (name, start) in [
        ("start", body("start")),
        ("begin_start", body("begin_start")),
    ] {
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
            assert!(!start.contains(word), "{word:?} in {name}");
        }
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
    for (name, file) in [("main.rs", code(MAIN)), ("notify.rs", code(NOTIFY))] {
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
    for (name, file) in [("main.rs", code(MAIN)), ("notify.rs", code(NOTIFY))] {
        for call in file.split("add_css_class(").skip(1) {
            let call = call.trim_start();
            assert!(
                call.starts_with('"') || call.starts_with("match decision"),
                "{name}: add_css_class({call:.40}"
            );
        }
    }
}

#[test]
fn the_panel_has_no_way_to_rebuild_the_system() {
    // The "Run just rebuild" button was removed: applying changes stays a
    // terminal action. Nothing in the panel names it, spawns a terminal, or
    // runs anything but the two fixed read-only collectors.
    let manifest = include_str!("../Cargo.toml");
    for (name, text) in [
        ("main.rs", code(MAIN)),
        ("notify.rs", code(NOTIFY)),
        ("style.rs", code(STYLE)),
        ("Cargo.toml", manifest.to_owned()),
    ] {
        for word in [
            "vte",
            "Vte",
            "mod rebuild",
            "Rebuild",
            "rebuild_",
            "\"just\"",
            "just rebuild",
            "nixos-rebuild",
            "spawn_async",
            "spawn_command_line",
            "AppInfo",
            "launch_uri",
            "Terminal",
            "diagnose-rebuild",
            "rebuild-failed",
        ] {
            assert!(!text.contains(word), "{word:?} in {name}");
        }
    }
    // The only process the panel starts is the fixed journalctl/coredumpctl
    // argv from the view-model, in notify.rs; main.rs starts none.
    assert_eq!(code(NOTIFY).matches("Subprocess::newv(").count(), 1);
    assert!(!code(MAIN).contains("Subprocess"));
    assert!(!code(MAIN).contains("Command::new"));
    assert!(!code(NOTIFY).contains("Command::new"));
    // The view-model has no state machine for it either.
    for (name, file) in VIEWMODEL {
        let code = shipped(file);
        for word in ["RebuildCommand", "describe_wait_status", "/etc/nixos"] {
            assert!(!code.contains(word), "{word:?} in view-model {name}");
        }
    }
}

#[test]
fn the_event_loop_denies_too_large_requests_through_apply_with() {
    // A request too large to review is denied at once by `apply_with`; a plain
    // `apply` would leave it to the core's timeout. So the loop calls
    // `apply_with` whenever there is a session, and `apply` only when there is
    // none (nothing to answer to).
    let main = code(MAIN);
    assert!(main.contains("Some(session) => c.apply_with(event, &**session),"));
    assert_eq!(main.matches(".apply(").count(), 1);
    assert!(main.contains("None => c.apply(event),"));
    // and the panel says what happened, in fixed words
    assert!(main.contains("Answer::TooLarge => \"Denied: too large to review.\","));
    assert!(main.contains("Proposal::TooLarge(bytes)"));
    assert!(main.contains("This request is too large to review here"));
}

#[test]
fn a_tool_card_is_named_only_through_the_view_model() {
    // The id is agent text; `ToolCard::name` shows it as visible text.
    let main = code(MAIN);
    assert!(main.contains("let name = card.name();"));
    assert!(!main.contains("&card.id"));
}

#[test]
fn a_click_never_leaves_send_focused() {
    // A focused Send is activated by Enter or Space, and unlike Enter in the
    // input it sends a draft that has not been edited.
    let main = code(MAIN);
    let from = main
        .find("let send = gtk::Button::builder()")
        .expect("the send button");
    let builder = &main[from..];
    let builder = &builder[..builder.find(".build();").expect("its build")];
    assert!(builder.contains(".focus_on_click(false)"), "{builder}");
    assert!(!main.contains("set_focus_on_click(true)"));
}

#[test]
fn send_fills_the_input_row_with_a_fixed_class() {
    let main = code(MAIN);
    assert!(main.contains("send.add_css_class(\"vexos-send\");"));
    assert!(main.contains("send.set_valign(gtk::Align::Fill);"));
    assert!(!main.contains("send.set_valign(gtk::Align::End)"));
    assert!(include_str!("style.css").contains("button.vexos-send {"));
}

#[test]
fn launch_timing_reads_only_the_process_clock_and_the_hook_does_nothing_else() {
    let main = code(MAIN);
    // The process age is read from two /proc files, in one place, read only.
    for path in ["\"/proc/self/stat\"", "\"/proc/uptime\""] {
        assert_eq!(main.matches(path).count(), 1, "{path}");
        for (name, file) in [("notify.rs", NOTIFY), ("style.rs", STYLE)] {
            assert!(!code(file).contains(path), "{path} in {name}");
        }
        for (name, file) in VIEWMODEL {
            assert!(!shipped(file).contains(path), "{path} in {name}");
        }
    }
    assert_eq!(main.matches("fs::read_to_string(\"/proc").count(), 2);
    assert_eq!(main.matches("since_process_start(").count(), 1);
    // Only the first window of a process gets the process timeline: the cell
    // is emptied, not copied.
    assert_eq!(main.matches("STARTUP.take()").count(), 1);
    assert_eq!(main.matches("STARTUP.with(").count(), 1); // `mark`
    // The first-frame hook records the time and then calls `begin_start` (the
    // one thing that starts the agent, pinned in the test below); it names no
    // composer, prompt, session, agent, runtime or file itself.
    let from = main.find("fn time_first_frame(").expect("hook");
    let hook = &main[from..];
    let hook = &hook[..hook.find("\n}\n").expect("end of hook")];
    assert_eq!(hook.matches("connect_after_paint(").count(), 1);
    assert_eq!(hook.matches("connect_map(").count(), 1);
    for word in [
        "submit",
        "send_prompt",
        "composer",
        "prompt(",
        "session",
        "Session",
        "agent",
        "runtime()",
        "spawn",
        "fs::",
        "File",
    ] {
        assert!(!hook.contains(word), "{word:?} in time_first_frame");
    }
    // Both hooks run for the one window `window()` builds, and the callers
    // that open the window do not time anything themselves.
    assert_eq!(main.matches("time_first_frame(&ui)").count(), 1);
}

#[test]
fn the_agent_starts_after_the_first_frame_or_at_the_deadline_and_never_twice() {
    let main = code(MAIN);
    // `start` only prepares: the one spawn is in `begin_start`.
    let start = body("start");
    assert!(!start.contains("Session::spawn(") && !start.contains("runtime()"));
    let begin = body("begin_start");
    assert_eq!(begin.matches("Session::spawn(").count(), 1);
    // Once: the pending agent is taken first, so a second call does nothing.
    assert!(
        begin.contains("let Some(agent) = self.pending_start.take() else {"),
        "{begin}"
    );
    assert!(begin.find("pending_start.take()") < begin.find("Session::spawn("));
    // Set in one place, cleared on close.
    assert_eq!(main.matches("pending_start.set(Some(").count(), 1);
    // The first thing `close_session` does, before its early return.
    let close = body("close_session");
    assert!(
        close.contains("{\n        self.pending_start.set(None);\n        let Some(session)"),
        "{close}"
    );
    // Reached from the deadline timer, the first-frame hook and the prewarm
    // action (a window that is never shown never paints), nowhere else.
    assert_eq!(main.matches("begin_start()").count(), 3);
    assert!(start.contains("glib::timeout_add_local_once(START_DEADLINE"));
    assert!(start.contains("ui.begin_start();"));
    let from = main.find("fn time_first_frame(").expect("hook");
    let hook = &main[from..];
    let hook = &hook[..hook.find("\n}\n").expect("end of hook")];
    assert_eq!(hook.matches("ui.begin_start();").count(), 1);
    // After the first frame was recorded.
    let (frame, begin) = (hook.find("c.first_frame("), hook.find("ui.begin_start();"));
    assert!(frame.is_some() && begin.is_some() && frame < begin);
    // The frame handler disconnects itself, or it would run on every frame.
    assert_eq!(hook.matches("clock.disconnect(id)").count(), 1);
    // ... and it is given the id to disconnect, or `take()` finds nothing.
    assert_eq!(hook.matches("handler.set(Some(id))").count(), 1);
    assert!(main.contains("const START_DEADLINE: Duration = Duration::from_secs(3);"));
}

const TERMINAL: &str = include_str!("terminal.rs");

#[test]
fn the_extension_can_only_fill_the_input_start_over_or_open_a_terminal() {
    let main = code(MAIN);
    // The app actions, all of them: the window toggle, a starter's draft, a
    // fresh conversation, the terminal and prewarm. (The notification actions
    // are in notify.rs.)
    assert_eq!(main.matches("gio::ActionEntry::builder(").count(), 5);
    for name in ["toggle", "draft", "new-session", "terminal", "prewarm"] {
        assert!(
            main.contains(&format!("gio::ActionEntry::builder(\"{name}\")")),
            "{name}"
        );
    }
    // A draft: a known starter's fixed prompt goes into the input as a draft,
    // which needs a click on Send. Nothing is sent from here.
    let from = main
        .find("gio::ActionEntry::builder(\"draft\")")
        .expect("draft action");
    let draft = &main[from..from + main[from..].find(".build();").expect("its end")];
    assert!(draft.contains(".and_then(starter)"));
    assert!(draft.contains("ui.hold_draft(found.prompt.to_owned());"));
    for word in ["submit", "send_prompt", "prompt(", "Outgoing", "session"] {
        assert!(!draft.contains(word), "{word:?} in the draft action");
    }
    // The terminal: one constant program, no arguments, no shell.
    let terminal = code(TERMINAL);
    assert_eq!(terminal.matches("Subprocess::newv(").count(), 1);
    assert!(terminal.contains("const PROGRAM: &str = \"vexos-ai\";"));
    assert!(terminal.contains("&[OsStr::new(PROGRAM)]"));
    for word in [
        "format!",
        "String",
        "args",
        "\"-c\"",
        "sh ",
        "spawn_command_line",
        "Command::new",
        "env",
    ] {
        assert!(!terminal.contains(word), "{word:?} in terminal.rs");
    }
    assert!(main.contains(".activate(|_: &adw::Application, _, _| terminal::open())"));
    // Nothing else in main.rs starts a process.
    assert!(!main.contains("Subprocess") && !main.contains("Command::new"));
}

#[test]
fn closing_the_window_hides_it_and_only_end_destroys_it() {
    let main = code(MAIN);
    // The Close button hides the window and keeps the session.
    assert_eq!(main.matches("connect_close_request(").count(), 1);
    let from = main.find("connect_close_request(").expect("the handler");
    let handler = &main[from..from + main[from..].find("\n        });").expect("its end")];
    assert!(handler.contains("ui.hide();"));
    assert!(handler.contains("glib::Propagation::Stop"));
    assert!(!handler.contains("Proceed") && !handler.contains("close_session"));
    // Only `end` destroys the window, after it has denied what is pending.
    assert_eq!(main.matches(".destroy()").count(), 1);
    let end = body("end");
    assert!(end.find("self.close_session();") < end.find("self.window.destroy();"));
    assert!(end.contains("self.window.destroy();"));
    // `end` is reached from the new-session action, the menu's "End session"
    // and the hidden-too-long timer, nowhere else.
    assert_eq!(main.matches(".end()").count(), 3);
    let hide = body("hide");
    assert!(hide.contains("HIDDEN_LIMIT") && hide.contains("if !ui.window.is_visible() {"));
    assert!(main.contains("const HIDDEN_LIMIT: Duration = Duration::from_secs(10 * 60);"));
    // Showing the window cancels the timer.
    assert!(main.contains("ui.cancel_hidden_timer();"));
}

#[test]
fn a_starter_card_only_fills_the_input() {
    let main = code(MAIN);
    // The card's click puts the fixed prompt in the input as a draft, which
    // needs a click on Send. It cannot send.
    assert!(main.contains("button.connect_clicked(move |_| ui.hold_draft(prompt.to_owned()));"));
    assert_eq!(main.matches("hold_draft(").count(), 3); // the action, the card, the method
    assert!(main.contains("for (button, prompt) in starter_buttons {"));
    assert!(main.contains(".map(|s| {") && main.contains("(button, s.prompt)"));
    // The prompts are the view-model's constants, never built here.
    assert!(!main.contains("format!(\"{}\", s.prompt"));
}

#[test]
fn prewarm_builds_the_window_unshown_and_sends_nothing() {
    let main = code(MAIN);
    let from = main
        .find("gio::ActionEntry::builder(\"prewarm\")")
        .expect("prewarm action");
    let prewarm = &main[from..from + main[from..].find(".build();").expect("its end")];
    // No parameter: a caller can warm the app and nothing else.
    assert!(!prewarm.contains("parameter_type"));
    // With a window already open it does nothing; otherwise it builds the
    // window, starts the agent and hides it, which arms the hidden timer.
    assert!(prewarm.contains("if slot.borrow().is_some() {"));
    assert!(prewarm.contains("ui.begin_start();") && prewarm.contains("ui.hide();"));
    assert!(prewarm.find("ui.begin_start();") < prewarm.find("ui.hide();"));
    // It shows nothing and sends nothing.
    for word in [
        "present",
        "submit",
        "send_prompt",
        "prompt(",
        "Outgoing",
        "hold_draft",
        "composer",
    ] {
        assert!(!prewarm.contains(word), "{word:?} in the prewarm action");
    }
}

#[test]
fn a_streamed_reply_is_updated_in_place_and_followed_after_layout() {
    let main = code(MAIN);
    // Drawing a row never scrolls by itself: the list follows `upper` after
    // layout, and only while the user is at the end.
    let render = body("render");
    assert!(!render.contains("set_value") && !render.contains("upper()"));
    assert!(main.contains("adjustment.connect_upper_notify("));
    assert!(
        main.contains(
            "adjustment.set_value(end_value(adjustment.upper(), adjustment.page_size()));"
        )
    );
    // Following is lost only when the user was acting on the list; a move
    // with no input behind it is put back (`Moved::Restore`). Input is seen by
    // a raw capture controller, because a gesture start and end are unreliable.
    assert!(main.contains("Moved::Restore => adjustment.set_value(end_value(upper, page)),"));
    assert!(main.contains("Moved::Resume => ui.follow.set(true),"));
    assert!(main.contains("Moved::Stop => ui.follow.set(false),"));
    assert!(main.contains("gtk::EventControllerLegacy::new()"));
    assert!(main.contains("ui.scroller.add_controller(input);"));
    assert_eq!(main.matches("ui.follow.set(false)").count(), 1);
    // A grown reply is filled into its existing view; replacing the widget
    // would shrink the list and throw the scroll position.
    let update = body("update");
    assert!(update.contains("fill_reply(&view, &text, &TagColors::of(&self.window));"));
    assert!(!update.contains("self.list.remove("));
    // Sending asks to see the answer.
    assert!(body("send_prompt").contains("self.follow.set(true);"));
}

#[test]
fn replacing_a_row_never_lets_the_focus_fall_to_the_top_of_the_list() {
    // Removing the row that holds the focus let it fall to a label at the top,
    // and the scroller followed it there. Rows are swapped in one place, which
    // moves the focus to the scroller first; the viewport keeps scrolling to
    // the focus for keyboard users.
    let main = code(MAIN);
    assert!(main.contains("swap_row(&self.list, &self.scroller, old, &widget);"));
    assert!(!main.contains("self.list.remove(old)"));
    assert!(!main.contains("set_scroll_to_focus(false)"));
    // `swap_row` is a free function: up to its closing brace at column 0.
    let swap = &main[main.find("fn swap_row(").expect("swap_row")..];
    let swap = &swap[..swap.find("\n}\n").expect("end of swap_row")];
    assert!(swap.contains("scroller.grab_focus();"));
    assert!(swap.find("scroller.grab_focus();") < swap.find("list.remove(old);"));
}
