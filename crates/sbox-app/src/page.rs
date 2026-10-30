//! A Razor page on screen: the parsed document, its bound values and the game-code handlers its `onclick=@Name` buttons
//! call. A handler changes the values; the page is laid out again and replaces its widgets in the UI.

use std::collections::HashMap;
use std::ops::Range;

use crate::razor::Document;
use crate::razor_ui::{self, Item};
use crate::ui::{self, Ui};
use sbox_render::Renderer;

/// What a click on a page button asks the app to do besides changing values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
}

pub type Handler = Box<dyn FnMut(&mut HashMap<String, String>) -> Effect>;

pub struct Page {
    doc: Document,
    pub values: HashMap<String, String>,
    /// Opacity and scale from the scene's `ScreenPanel` (1, 1 without one).
    pub opacity: f32,
    pub scale: f32,
    handlers: HashMap<String, Handler>,
    /// Where the page's widgets sit in the UI vectors, so a re-layout replaces them and leaves the rest alone.
    panels: Range<usize>,
    labels: Range<usize>,
    buttons: Range<usize>,
}

impl Page {
    pub fn new(doc: Document, values: HashMap<String, String>) -> Self {
        let mut p = Self { doc, values, opacity: 1.0, scale: 1.0, handlers: HashMap::new(), panels: 0..0, labels: 0..0, buttons: 0..0 };
        p.register_defaults();
        p
    }

    /// Handlers every page gets: `Quit`, and `Increment` / `Decrement` / `Toggle` acting on the `Count` and `Show` values.
    fn register_defaults(&mut self) {
        self.on("Quit", |_| Effect::Quit);
        self.on("Increment", |v| {
            bump(v, "Count", 1);
            Effect::None
        });
        self.on("Decrement", |v| {
            bump(v, "Count", -1);
            Effect::None
        });
        self.on("Toggle", |v| {
            let on = v.get("Show").is_some_and(|s| s == "true");
            v.insert("Show".into(), (!on).to_string());
            Effect::None
        });
    }

    /// Game code for the button whose `onclick` is `@name`; replaces an earlier handler of that name.
    pub fn on(&mut self, name: &str, handler: impl FnMut(&mut HashMap<String, String>) -> Effect + 'static) {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    /// Run the handler for a clicked button id. `None` when the page has no handler of that name.
    pub fn click(&mut self, id: &str) -> Option<Effect> {
        let handler = self.handlers.get_mut(id)?;
        Some(handler(&mut self.values))
    }

    /// (Re)build the page's widgets: whatever an earlier call added is removed first.
    pub fn populate(&mut self, ui: &mut Ui, r: &mut Renderer, font: &ab_glyph::FontVec) {
        ui.panels.drain(self.panels.clone());
        ui.labels.drain(self.labels.clone());
        ui.buttons.drain(self.buttons.clone());
        let (k, a) = (self.scale, self.opacity);
        // Laid out at 1x and then scaled, so the page keeps its proportions.
        let (items, content) = razor_ui::layout_bound(&self.doc, 40.0, 40.0, 520.0, &self.values);
        ui.content_height = (40.0 + content + 40.0) * k;
        let r4 = |r: [f32; 4]| [r[0] * k, r[1] * k, r[2] * k, r[3] * k];
        let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * a];
        // The page is always the first thing in the UI after the HUD, so appending is where its old widgets were.
        self.panels = ui.panels.len()..ui.panels.len();
        self.labels = ui.labels.len()..ui.labels.len();
        self.buttons = ui.buttons.len()..ui.buttons.len();
        for item in items {
            match item {
                Item::Panel { rect, color } => ui.panels.push(ui::Panel { rect: r4(rect), color: fade(color) }),
                Item::Text { text, size, color, rect } => {
                    let (texture, [w, h]) = crate::App::text_texture(r, font, &text, size * k);
                    ui.labels.push(ui::Label { texture, rect: [rect[0] * k, rect[1] * k, w, h], tint: fade(color) });
                }
                Item::Button { id, text, rect, color } => ui.buttons.push(ui::Button {
                    id,
                    rect: r4(rect),
                    color: fade(color.unwrap_or([0.18, 0.2, 0.3, 1.0])),
                    hover_color: fade([0.3, 0.4, 0.7, 1.0]),
                    label: Some(crate::App::text_texture(r, font, &text, 24.0 * k)),
                }),
            }
        }
        self.panels.end = ui.panels.len();
        self.labels.end = ui.labels.len();
        self.buttons.end = ui.buttons.len();
    }
}

fn bump(v: &mut HashMap<String, String>, key: &str, by: i64) {
    let n = v.get(key).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    v.insert(key.to_string(), (n + by).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(src: &str) -> Page {
        Page::new(crate::razor::parse(src).unwrap(), HashMap::new())
    }

    #[test]
    fn clicks_run_game_code_that_changes_the_values() {
        let mut p = page("<root><button onclick=@Increment>+</button>@Count</root>");
        assert_eq!(p.click("Increment"), Some(Effect::None));
        assert_eq!(p.click("Increment"), Some(Effect::None));
        p.click("Decrement");
        assert_eq!(p.values["Count"], "1");
        assert_eq!(p.click("Quit"), Some(Effect::Quit));
        assert_eq!(p.click("Nope"), None, "unknown handlers are left to the app");
        p.click("Toggle");
        assert_eq!(p.values["Show"], "true");
        p.click("Toggle");
        assert_eq!(p.values["Show"], "false");
    }

    #[test]
    fn screen_panel_settings_default_to_identity() {
        let p = page("<root/>");
        assert_eq!((p.opacity, p.scale), (1.0, 1.0));
    }

    #[test]
    fn custom_handlers_replace_defaults() {
        let mut p = page("<root/>");
        p.on("Increment", |v| {
            v.insert("Count".into(), "100".into());
            Effect::None
        });
        p.click("Increment");
        assert_eq!(p.values["Count"], "100");
    }

    #[test]
    fn changed_values_change_the_layout() {
        let mut p = page("<root><p>@if (Show) {<b>on</b>} else {<i>off</i>}</p></root>");
        let text = |p: &Page| -> Vec<String> {
            razor_ui::layout_bound(&p.doc, 0.0, 0.0, 400.0, &p.values).0.into_iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text) } else { None }).collect()
        };
        assert_eq!(text(&p), vec!["off"]);
        p.click("Toggle");
        assert_eq!(text(&p), vec!["on"]);
    }
}
