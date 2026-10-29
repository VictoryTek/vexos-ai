//! The panel's window mode, shared with the Shell extension through one
//! GSettings key. It holds one of two fixed strings and nothing else: no agent
//! text, command or path.

/// The extension's own schema, installed for the panel too (nix/panel.nix).
pub const WINDOW_SCHEMA: &str = "org.gnome.shell.extensions.vexos-assistant";
pub const WINDOW_MODE_KEY: &str = "window-mode";
/// Owned on the session bus by the extension while it is enabled.
pub const EXTENSION_BUS_NAME: &str = "io.github.victorytek.VexosAssistant.Extension";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Docked,
    Floating,
}

impl WindowMode {
    /// Exactly `"floating"` is floating; anything else is the default, docked.
    pub fn parse(value: &str) -> WindowMode {
        if value == "floating" {
            WindowMode::Floating
        } else {
            WindowMode::Docked
        }
    }

    /// The only strings ever written.
    pub fn as_str(self) -> &'static str {
        match self {
            WindowMode::Docked => "docked",
            WindowMode::Floating => "floating",
        }
    }

    pub fn toggled(self) -> WindowMode {
        match self {
            WindowMode::Docked => WindowMode::Floating,
            WindowMode::Floating => WindowMode::Docked,
        }
    }
}

/// What the header button and the window's look should be.
#[derive(Debug, PartialEq, Eq)]
pub struct WindowLook {
    pub toggle_visible: bool,
    /// Square top corners (the extension has docked the window).
    pub docked: bool,
    pub icon: &'static str,
    pub tooltip: &'static str,
}

/// The toggle shows only with the extension running and the schema found;
/// otherwise the window is the plain one.
pub fn window_look(extension_running: bool, schema_found: bool, mode: WindowMode) -> WindowLook {
    let active = extension_running && schema_found;
    let docked = active && mode == WindowMode::Docked;
    WindowLook {
        toggle_visible: active,
        docked,
        icon: if docked {
            "view-restore-symbolic"
        } else {
            "go-top-symbolic"
        },
        tooltip: if docked {
            "Switch to a floating window"
        } else {
            "Dock under the top bar"
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_string_floating_is_floating() {
        assert_eq!(WindowMode::parse("floating"), WindowMode::Floating);
        assert_eq!(WindowMode::parse("docked"), WindowMode::Docked);
        let long = "floating".repeat(200_000);
        for bad in [
            "",
            "bogus",
            " floating",
            "floating ",
            "FLOATING",
            "floating\n",
            "floating\0",
            "\u{1b}[31mfloating",
            "docked\u{7}",
            &long,
        ] {
            assert_eq!(WindowMode::parse(bad), WindowMode::Docked, "{bad:?}");
        }
    }

    #[test]
    fn only_two_strings_are_ever_written_and_they_round_trip() {
        for mode in [WindowMode::Docked, WindowMode::Floating] {
            assert_eq!(WindowMode::parse(mode.as_str()), mode);
        }
        assert_eq!(WindowMode::Docked.as_str(), "docked");
        assert_eq!(WindowMode::Floating.as_str(), "floating");
    }

    #[test]
    fn toggle_switches_both_ways() {
        assert_eq!(WindowMode::Docked.toggled(), WindowMode::Floating);
        assert_eq!(WindowMode::Floating.toggled(), WindowMode::Docked);
    }

    #[test]
    fn the_toggle_needs_the_extension_and_the_schema() {
        for mode in [WindowMode::Docked, WindowMode::Floating] {
            for (running, schema) in [(false, false), (true, false), (false, true)] {
                let look = window_look(running, schema, mode);
                assert!(!look.toggle_visible && !look.docked, "{running} {schema}");
            }
        }
        let docked = window_look(true, true, WindowMode::Docked);
        assert!(docked.toggle_visible && docked.docked);
        let floating = window_look(true, true, WindowMode::Floating);
        assert!(floating.toggle_visible && !floating.docked);
        assert_ne!(docked.tooltip, floating.tooltip);
        assert_ne!(docked.icon, floating.icon);
    }
}
