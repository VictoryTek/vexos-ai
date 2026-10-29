//! The panel's look: one static stylesheet, bundled into the binary, plus the
//! colours of text tags (which CSS cannot reach), derived from the theme.
//! Nothing here is built from agent text, usernames, paths or any other run
//! time content: the class names in the code are fixed strings.

use adw::prelude::*;
use gtk::gdk;

pub const STYLESHEET: &str = include_str!("style.css");

/// Loads the stylesheet for the whole display, once.
pub fn load(display: &gdk::Display) {
    let css = gtk::CssProvider::new();
    // GTK hands the high-contrast setting only to its own theme providers; a
    // provider the app makes reports "no preference" unless it is bound, and
    // then `@media (prefers-contrast: more)` in the sheet never matches.
    gtk::Settings::for_display(display)
        .bind_property("gtk-interface-contrast", &css, "prefers-contrast")
        .sync_create()
        .build();
    css.load_from_string(STYLESHEET);
    gtk::style_context_add_provider_for_display(
        display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Colours for the text tags, from the window's foreground colour and the
/// style's light or dark setting. Re-read when either changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TagColors {
    /// The tint behind code and tables.
    pub code_background: gdk::RGBA,
    /// Diff hunk headers.
    pub hunk: gdk::RGBA,
    pub added: gdk::RGBA,
    pub removed: gdk::RGBA,
}

/// libadwaita's own success and destructive background colours for the light
/// and the dark style; `rgba_to_standalone` turns them into text colours with
/// enough contrast on that style's window background.
const SUCCESS: [(f32, f32, f32); 2] = [(0.18, 0.76, 0.49), (0.15, 0.64, 0.41)];
const DESTRUCTIVE: [(f32, f32, f32); 2] = [(0.88, 0.11, 0.14), (0.75, 0.11, 0.16)];

fn with_alpha(color: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}

impl TagColors {
    /// `standalone` turns a background colour into a text colour for the
    /// style; the panel passes libadwaita's own function.
    pub fn new(
        foreground: &gdk::RGBA,
        dark: bool,
        standalone: impl Fn(&gdk::RGBA, bool) -> gdk::RGBA,
    ) -> TagColors {
        let pick = |pair: [(f32, f32, f32); 2]| {
            let (r, g, b) = pair[usize::from(dark)];
            standalone(&gdk::RGBA::new(r, g, b, 1.0), dark)
        };
        TagColors {
            code_background: with_alpha(foreground, if dark { 0.12 } else { 0.07 }),
            hunk: with_alpha(foreground, 0.65),
            added: pick(SUCCESS),
            removed: pick(DESTRUCTIVE),
        }
    }

    /// For a widget that is in a window: its foreground colour and the
    /// current style.
    pub fn of(widget: &impl IsA<gtk::Widget>) -> TagColors {
        TagColors::new(
            &widget.color(),
            adw::StyleManager::default().is_dark(),
            adw::rgba_to_standalone,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const MAIN: &str = include_str!("main.rs");
    const NOTIFY: &str = include_str!("notify.rs");
    const REBUILD: &str = include_str!("rebuild.rs");

    /// Style classes from libadwaita and GTK that the code may use.
    const LIBRARY: [&str; 12] = [
        "flat",
        "card",
        "caption",
        "heading",
        "monospace",
        "numeric",
        "dimmed",
        "dim-label",
        "suggested-action",
        "destructive-action",
        "title-3",
        "title-2",
    ];

    /// Every string literal given to `add_css_class(` in `source`. A call
    /// whose argument is not a literal is returned as `<not a literal>`.
    fn classes_in(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        for call in source.split("add_css_class(").skip(1) {
            let call = call.trim_start();
            if let Some(rest) = call.strip_prefix('"') {
                out.push(rest.split('"').next().unwrap_or_default().to_owned());
            } else if call.starts_with("match") {
                // `match decision { A => "x", B => "y" }`: the literals up to
                // the end of the call.
                let end = call.find(");").unwrap_or(call.len());
                let literals: Vec<&str> = call[..end].split('"').skip(1).step_by(2).collect();
                assert!(!literals.is_empty(), "{call}");
                out.extend(literals.into_iter().map(str::to_owned));
            } else {
                out.push("<not a literal>".to_owned());
            }
        }
        out
    }

    /// Class names defined in the stylesheet: `.name` selectors.
    fn defined() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for part in STYLESHEET.split('.').skip(1) {
            let name: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if name.starts_with("vexos-") {
                out.insert(name);
            }
        }
        out
    }

    fn used() -> BTreeSet<String> {
        [MAIN, NOTIFY, REBUILD]
            .into_iter()
            .flat_map(classes_in)
            .collect()
    }

    #[test]
    fn every_class_the_code_adds_is_defined_or_from_libadwaita() {
        let defined = defined();
        for class in used() {
            assert!(
                LIBRARY.contains(&class.as_str()) || defined.contains(&class),
                "style class {class:?} is neither in style.css nor a libadwaita class"
            );
        }
    }

    #[test]
    fn every_panel_class_in_the_stylesheet_is_used() {
        let used = used();
        for class in defined() {
            // `vexos-docked` is toggled with the literal in `apply_look`.
            assert!(
                used.contains(&class),
                "style.css defines .{class}, which the code never adds"
            );
        }
    }

    #[test]
    fn class_names_are_fixed_literals() {
        for file in [MAIN, NOTIFY, REBUILD] {
            assert!(
                !classes_in(file).contains(&"<not a literal>".to_owned()),
                "a style class is not a string literal"
            );
            for word in ["remove_css_class(", "set_css_classes("] {
                for call in file.split(word).skip(1) {
                    assert!(call.trim_start().starts_with('"'), "{word}{call:.30}");
                }
            }
        }
    }

    #[test]
    fn the_stylesheet_loads_nothing_and_has_no_templates() {
        for word in ["@import", "url(", "image(", "image-set(", "{{", "%s", "{}"] {
            assert!(!STYLESHEET.contains(word), "{word}");
        }
    }

    #[test]
    fn colours_come_from_the_theme_not_from_a_palette() {
        // Every colour in the sheet is a libadwaita variable or transparent.
        for word in ["#", "rgb(", "rgba(", "hsl("] {
            assert!(!STYLESHEET.contains(word), "{word}");
        }
    }

    /// Needs a display, so it is skipped by default. Run it against a
    /// headless or nested session: `cargo test -- --ignored`. Fails on any
    /// CSS parsing error, which GTK otherwise only prints.
    #[test]
    #[ignore = "needs a display"]
    fn the_stylesheet_parses_without_errors() {
        gtk::init().expect("a display");
        let css = gtk::CssProvider::new();
        let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        css.connect_parsing_error({
            let errors = errors.clone();
            move |_, section, error| {
                errors
                    .borrow_mut()
                    .push(format!("{}: {error}", section.start_location().lines()));
            }
        });
        css.load_from_string(STYLESHEET);
        assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    }

    #[test]
    fn tag_colours_follow_the_foreground_and_the_style() {
        let ink = gdk::RGBA::new(0.1, 0.2, 0.3, 1.0);
        // libadwaita's function needs a display; this one only has to show
        // that the style flag reaches it and picks the matching base colour.
        let fake = |base: &gdk::RGBA, dark: bool| {
            let k = if dark { 1.0 } else { 0.0 };
            gdk::RGBA::new(base.red() * 0.5 + 0.5 * k, base.green(), base.blue(), 1.0)
        };
        let light = TagColors::new(&ink, false, fake);
        let dark = TagColors::new(&ink, true, fake);
        assert!((light.code_background.alpha() - 0.07).abs() < 1e-6);
        assert!((dark.code_background.alpha() - 0.12).abs() < 1e-6);
        assert_eq!(light.code_background.green(), 0.2);
        assert_eq!(light.hunk.red(), 0.1);
        assert_eq!(light.hunk.blue(), 0.3);
        assert!((light.hunk.alpha() - 0.65).abs() < 1e-6);
        assert_eq!(dark.hunk.red(), 0.1);
        // The dark style gets the dark base colour and the dark flag.
        assert!((light.added.red() - (0.18 * 0.5)).abs() < 1e-6);
        assert!((dark.added.red() - (0.15 * 0.5 + 0.5)).abs() < 1e-6);
        assert_eq!(light.added.green(), 0.76);
        assert_eq!(dark.added.green(), 0.64);
        assert!((light.removed.red() - 0.88 * 0.5).abs() < 1e-6);
        assert!((dark.removed.red() - (0.75 * 0.5 + 0.5)).abs() < 1e-6);
        assert_ne!(light.added, light.removed);
    }
}
