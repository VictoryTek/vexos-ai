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
        ] {
            assert!(!file.contains(word), "{word:?} in notification code");
        }
    }
    assert!(NOTIFY.contains("ui.hold_draft(draft("));
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
