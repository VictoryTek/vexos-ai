//! Source checks for what the GTK code can't be run headless to show: a
//! notification never sends a prompt or starts a session. Kept in its own
//! file so the words it looks for don't appear in the files it reads.

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

#[test]
fn notification_code_never_sends_or_starts_a_session() {
    for file in [code(NOTIFY), code(REBUILD)] {
        for word in [
            "send_prompt",
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
    // What a notification action does with the window: present it and hold
    // a draft.
    assert!(NOTIFY.contains("ui.hold_draft(draft("));
}

#[test]
fn only_send_and_enter_send_a_prompt() {
    // Defined once, called once: from `on_send`, which only the Send button
    // and the Enter key call.
    assert_eq!(MAIN.matches("send_prompt").count(), 2);
    assert_eq!(MAIN.matches(".send_prompt()").count(), 1);
    assert!(MAIN.contains("let on_send = {\n            let ui = ui.clone();\n            move || ui.send_prompt()\n        };"));
    assert_eq!(MAIN.matches("on_send()").count(), 2);
    assert_eq!(MAIN.matches("session.prompt(").count(), 1);
    // A held draft only ever becomes input text.
    let hold = &MAIN[MAIN.find("fn hold_draft").unwrap()..];
    let hold = &hold[..hold.find("\n    }\n").unwrap()];
    assert!(
        !hold.contains("send") && !hold.contains("prompt("),
        "{hold}"
    );
}
